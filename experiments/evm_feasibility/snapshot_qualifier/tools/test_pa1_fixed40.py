"""Offline selected40 compiler regressions; no database or runtime command."""
import unittest
import hashlib
from pathlib import Path
import tempfile
import json
from types import SimpleNamespace
from unittest import mock

import pa1_fixed40 as fixed


class CompleteProspectiveReviewReadTests(unittest.TestCase):
    """Future review models retain the full original review read closure."""
    def specimen(self,role=60,metadata=0):
        import pa1_fixed40_constructors as c
        def pin(name):
            return dict(path='/home/fry/artifacts/NONHISTORICAL-review-'+name,
                        bytes=1,sha256=hashlib.sha256(name.encode()).hexdigest())
        names=(('exact_commands','command_verification','freeze','execution_helper')
               if role==60 else ('lead_verification','independent_verification',
                                'reverse_outcome','fact_seal_inputs'))
        proof_names=(('preflight','candidate_review','candidate','prior_bundle','prior_report',
            'source_review','command_verification','command_closure','normal_hook','gate_equivalence')
            if role==60 else ('bundle','capture_report','lead_report','independent_report',
            'independent_verification','lead_verification','reverse_outcome','fact_seal_inputs'))
        evidence={name:pin(name) for name in names}
        sources=[pin('source-%03d'%i) for i in range(101)]
        gates=[dict(role=r,result=pin('gate-%d'%r),log=pin('log-%d'%r))
               for r in c.VALIDATION_RESULTS]
        proofs={name:pin('proof-'+name) for name in proof_names}
        bounds=dict(aggregate_bytes=471859200,manifest_bytes=262144,max_json_depth=8,
            max_json_nodes=8192,max_path_bytes=4096,other_total_bytes=31457280,
            reference_body_bytes=1048576,reference_count_per_record=32,
            reference_count_total=64,reference_total_bytes=1048576,
            semantic_allocation_bytes=2097152,source_tool_total_bytes=4194304,
            stream_chunk_bytes=65536,wrapper_bytes=131072)
        result=dict(schema='fixed40-prospective-independent-review-scope-v1',role=role,
            profile=c.FIXED40_PROFILE,verdict='NOT_EXECUTED',evidence=evidence,
            source_freeze=sources,required_gates=gates,record_bounds=bounds,
            role_maxima=list(c.ROLE_MAXIMA),proofs=proofs,
            body='NONHISTORICAL full future scope; no acceptance has executed.')
        def invoke(value,omit=False):
            raw=fixed.core.canonical(value)
            review=dict(path=pin('review')['path'],bytes=len(raw),
                        sha256=hashlib.sha256(raw).hexdigest())
            pins=[review,*evidence.values(),*sources,
                  *(p for row in gates for p in (row['result'],row['log'])),*proofs.values()]
            kinds=['evidence']*5+['source']*101+['evidence']*(28+len(proofs))
            rows=tuple((i,list(fixed._pin_tuple(p)),kind,'bytes')
                       for i,(p,kind) in enumerate(zip(pins,kinds)))
            if omit:rows=rows[:-1]
            unit=SimpleNamespace(invocation=SimpleNamespace(metadata_charge=lambda:metadata,operations=bytearray(2048)),
                declaration_charge=0,declarations=rows,consume=mock.Mock())
            answer=c.independent_review_document(role,maxima=c.ROLE_MAXIMA,
                review_raw=raw,review_pin=review,evidence=evidence,unit=unit,
                declarations=rows,live_bytes=0)
            return answer,unit,pins,kinds
        return result,invoke

    def test_complete_prospective_source_gate_and_proof_reads_remain_pending(self):
        for role,count in ((60,144),(79,142)):
            value,invoke=self.specimen(role);raw,unit,pins,kinds=invoke(value)
            self.assertTrue(raw.startswith(b'PENDING\n'))
            self.assertFalse(raw.startswith(b'READY\n'))
            self.assertEqual(unit.consume.call_count,count)
            for call,pin,kind in zip(unit.consume.call_args_list,pins,kinds):
                self.assertEqual(call.args[1],pin)
                self.assertEqual(call.kwargs['kind'],kind)

    def test_prospective_verdict_cannot_claim_acceptance(self):
        for verdict in ('READY','ACCEPT_FINAL_FAMILY'):
            value,invoke=self.specimen();value['verdict']=verdict
            with self.assertRaisesRegex(ValueError,'closed independent review'):
                invoke(value)

    def test_missing_proof_declaration_and_unknown_schema_reject(self):
        value,invoke=self.specimen()
        with self.assertRaisesRegex(ValueError,'original declarations'):invoke(value,True)
        value['schema']='unreviewed-scope'
        with self.assertRaisesRegex(ValueError,'closed independent review'):invoke(value)

    def test_full_review_closure_with_large_invocation_metadata(self):
        for role,count in ((60,144),(79,142)):
            # One million already exceeds the unchanged entry guard for this
            # full fixture. Exercise a large admissible owner separately.
            value,invoke=self.specimen(role,metadata=800000)
            raw,unit,pins,kinds=invoke(value)
            self.assertTrue(raw.startswith(b'PENDING\n'))
            self.assertEqual(unit.consume.call_count,count)

    def test_overlarge_review_metadata_still_rejects_at_original_entry_guard(self):
        for role in (60,79):
            value,invoke=self.specimen(role,metadata=1000000)
            with self.assertRaisesRegex(ValueError,'concrete constructor caller lifetime'):
                invoke(value)

    def test_actual_review_preserves_exact_original_verdict_and_body(self):
        for role,verdict in ((60,'READY'),(79,'ACCEPT_FINAL_FAMILY')):
            value,invoke=self.specimen(role)
            value.update(schema='fixed40-independent-review-result-v1',verdict=verdict)
            raw,unit,pins,kinds=invoke(value)
            self.assertEqual(raw,verdict.encode()+b'\n'+fixed.core.canonical(value))
            self.assertEqual(unit.consume.call_count,144 if role==60 else 142)

class CurrentBinaryFitReadTests(unittest.TestCase):
    """The metadata mapping cannot substitute for either current binary read."""
    def specimen(self,consumer=False):
        import pa1_fixed40_constructors as constructors
        def pin(name,size=1):
            return dict(path='/home/fry/artifacts/NONHISTORICAL-binary-fit-'+name,
                        bytes=size,sha256=hashlib.sha256(name.encode()).hexdigest())
        binaries=[pin('live'),pin('replay')]
        kwargs=dict(binaries=binaries,embedded_executable=binaries[0],
                    preflight=pin('preflight'),source_freeze=pin('sources'))
        raw=constructors.binary_successor_mapping(maxima=constructors.ROLE_MAXIMA,
                                                  live_bytes=0,**kwargs)
        step=dict(role=34,constructor='binary_successor_mapping',maximum=16384,
                  output=dict(path=pin('output')['path'],bytes=len(raw),
                              sha256=hashlib.sha256(raw).hexdigest()))
        rows=[(i,list(fixed._pin_tuple(p)),'binary','bytes') for i,p in enumerate(binaries)]
        if consumer:rows.append((2,list(fixed._pin_tuple(step['output'])),'evidence','bytes'))
        unit=SimpleNamespace(invocation=SimpleNamespace(metadata_charge=lambda:0),
            declaration_charge=0,declarations=tuple(rows),consume=mock.Mock())
        return constructors,kwargs,raw,step,unit

    def test_producer_and_consumer_each_require_both_full_body_reads(self):
        for consumer in (False,True):
            c,kwargs,raw,step,unit=self.specimen(consumer)
            self.assertEqual(fixed._fit_construct(step,kwargs,{},unit,c.ROLE_MAXIMA,
                {},{},0,consumer=consumer,input_start=0,read_charge=None),raw)
            self.assertEqual(unit.consume.call_count,2)
            for call,pin in zip(unit.consume.call_args_list,kwargs['binaries']):
                self.assertEqual(call.args[1],pin)
                self.assertEqual(call.kwargs['kind'],'binary')
                self.assertGreater(call.kwargs['live_bytes'],0)

    def test_missing_reordered_or_reclassified_body_rejects_before_open(self):
        for case in ('missing','reordered','kind'):
            c,kwargs,raw,step,unit=self.specimen()
            if case=='missing':unit.declarations=()
            if case=='reordered':unit.declarations=tuple(reversed(unit.declarations))
            if case=='kind':
                unit.declarations=tuple((i,p,'evidence',mode) for i,p,k,mode in unit.declarations)
            with self.assertRaisesRegex(ValueError,'full body declarations'):
                fixed._fit_construct(step,kwargs,{},unit,c.ROLE_MAXIMA,
                    {},{},0,consumer=False,input_start=0,read_charge=None)
            unit.consume.assert_not_called()

    def test_original_unit_read_failure_cannot_publish_metadata_success(self):
        c,kwargs,raw,step,unit=self.specimen()
        unit.consume.side_effect=ValueError('original full hash/EOF failure')
        with self.assertRaisesRegex(ValueError,'original full hash/EOF failure'):
            fixed._fit_construct(step,kwargs,{},unit,c.ROLE_MAXIMA,
                {},{},0,consumer=False,input_start=0,read_charge=None)
        self.assertEqual(unit.consume.call_count,1)


class ColdGuardClaimTests(unittest.TestCase):
    """The administrative claim cannot read evidence or retry a failed start."""
    def test_absent_or_symlink_control_rejects_without_claim_or_body_read(self):
        from unittest import mock
        import pa1_fixed40_runtime as runtime
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'lock').write_bytes(b'')
            pin=dict(runtime.FIT_PROTECTED_COUNTER,path=str(root/'state.json'))
            opened=[];original=runtime.os.open
            def observe(path,*args,**kwargs):
                opened.append(str(path));return original(path,*args,**kwargs)
            with mock.patch.object(runtime,'FIT_PROTECTED_COUNTER',pin),mock.patch.object(
                    runtime.os,'open',side_effect=observe):
                with self.assertRaises(FileNotFoundError):
                    with runtime._claim_cold_guard(clock=lambda:1):pass
                (root/'target').write_bytes(b'{}')
                (root/'guard-bootstrap-control.json').symlink_to(root/'target')
                with self.assertRaisesRegex(ValueError,'bounded migration control'):
                    with runtime._claim_cold_guard(clock=lambda:1):pass
            self.assertFalse((root/'guard-bootstrap-claim.json').exists())
            self.assertFalse(any(p.endswith(('state.json','guard-bootstrap-control.json','target'))
                                 for p in opened))

    def test_one_claim_survives_failure_and_repeated_start_opens_no_body(self):
        from unittest import mock
        import pa1_fixed40_runtime as runtime
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'lock').write_bytes(b'')
            (root/'guard-bootstrap-control.json').write_bytes(b'{}')
            pin=dict(runtime.FIT_PROTECTED_COUNTER,path=str(root/'state.json'))
            with mock.patch.object(runtime,'FIT_PROTECTED_COUNTER',pin):
                with self.assertRaisesRegex(RuntimeError,'consumer failed'):
                    with runtime._claim_cold_guard(clock=lambda:5) as owner:
                        self.assertEqual(owner[3],305)
                        self.assertLessEqual((root/'guard-bootstrap-claim.json').stat().st_size,4096)
                        raise RuntimeError('consumer failed')
                claim=(root/'guard-bootstrap-claim.json').read_bytes()
                opened=[];original=runtime.os.open
                def observe(path,*args,**kwargs):
                    opened.append(str(path));return original(path,*args,**kwargs)
                with mock.patch.object(runtime.os,'open',side_effect=observe):
                    with self.assertRaises(FileExistsError):
                        with runtime._claim_cold_guard(clock=lambda:6):pass
                self.assertEqual((root/'guard-bootstrap-claim.json').read_bytes(),claim)
                self.assertFalse(any(p.endswith(('state.json','guard-bootstrap-control.json'))
                                     for p in opened))

    def test_external_deadline_expires_before_claim_without_an_extension(self):
        from unittest import mock
        import pa1_fixed40_runtime as runtime
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'lock').write_bytes(b'')
            (root/'guard-bootstrap-control.json').write_bytes(b'{}')
            pin=dict(runtime.FIT_PROTECTED_COUNTER,path=str(root/'state.json'))
            with mock.patch.object(runtime,'FIT_PROTECTED_COUNTER',pin):
                with self.assertRaisesRegex(ValueError,'external guard deadline'):
                    with runtime._claim_cold_guard(clock=mock.Mock(side_effect=[0,300])):pass
            self.assertFalse((root/'guard-bootstrap-claim.json').exists())

    def test_write_and_file_sync_failures_keep_a_durable_blocking_name(self):
        from unittest import mock
        import pa1_fixed40_runtime as runtime
        import stat
        for failure in ('write','file_sync'):
            with self.subTest(failure=failure),tempfile.TemporaryDirectory() as directory:
                root=Path(directory);(root/'lock').write_bytes(b'')
                (root/'guard-bootstrap-control.json').write_bytes(b'{}')
                pin=dict(runtime.FIT_PROTECTED_COUNTER,path=str(root/'state.json'))
                events=[];real_sync=runtime.os.fsync;real_write=runtime.os.write
                failed=False
                def sync(fd):
                    nonlocal failed
                    directory_fd=stat.S_ISDIR(runtime.os.fstat(fd).st_mode)
                    events.append('directory_sync' if directory_fd else 'file_sync')
                    if failure=='file_sync' and not directory_fd and not failed:
                        failed=True;raise OSError('injected file sync failure')
                    return real_sync(fd)
                def write(fd,raw):
                    events.append('write')
                    if failure=='write':raise OSError('injected write failure')
                    return real_write(fd,raw)
                with mock.patch.object(runtime,'FIT_PROTECTED_COUNTER',pin),mock.patch.object(
                        runtime.os,'fsync',side_effect=sync),mock.patch.object(
                        runtime.os,'write',side_effect=write):
                    with self.assertRaisesRegex(OSError,'injected'):
                        with runtime._claim_cold_guard(clock=lambda:1):pass
                    self.assertEqual(events[0],'directory_sync')
                    self.assertTrue((root/'guard-bootstrap-claim.json').exists())
                    with self.assertRaises(FileExistsError):
                        with runtime._claim_cold_guard(clock=lambda:2):pass


class GuardOriginReadTests(unittest.TestCase):
    """Cold reconstruction retains the original body and each full read debit."""
    def test_origin_alias_reads_complete_original_and_preserves_both_identities(self):
        from unittest import mock
        import pa1_fixed40_runtime as runtime
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);body=b'{"schema":2}\n'
            logical=dict(path=str(root/'state.json'),bytes=len(body),
                sha256=hashlib.sha256(body).hexdigest())
            physical=dict(logical,path=str(root/'guard-origin.json'))
            Path(physical['path']).write_bytes(body)
            # The live path now has different bytes. It must never supply the
            # mandatory pre-migration body during cold reconstruction.
            Path(logical['path']).write_bytes(b'{"schema":3}\n')
            admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
            admission._graphs=[];admission._outer_live=4096;admission.read_trace=[]
            charges=[];admission._full_read_charge=lambda packet:charges.append(packet)
            origin=(fixed._pin_tuple(logical),fixed._pin_tuple(physical))
            with mock.patch.object(runtime,'FIT_PROTECTED_COUNTER',logical):
                for unused in range(2):
                    self.assertEqual(admission._load(logical,4096,_origin=origin),len(body))
                self.assertEqual(charges,[(physical,),(physical,)])
                self.assertEqual([row['pin'] for row in admission.read_trace],
                    [fixed._pin_tuple(logical)]*2)
                self.assertEqual([row['physical_pin'] for row in admission.read_trace],
                    [fixed._pin_tuple(physical)]*2)
                # A truncated original is rejected after retaining its charge.
                Path(physical['path']).write_bytes(body[:-1])
                with self.assertRaisesRegex(ValueError,'truncated'):
                    admission._load(logical,4096,_origin=origin)
                self.assertEqual(len(charges),3)

    def test_origin_cannot_retarget_live_fit_or_any_other_input(self):
        from unittest import mock
        import pa1_fixed40_runtime as runtime
        origin=dict(runtime.FIT_PROTECTED_COUNTER,path=str(
            Path(runtime.FIT_PROTECTED_COUNTER['path']).parent/'guard-origin.json'))
        logical=fixed._pin_tuple(runtime.FIT_PROTECTED_COUNTER)
        with mock.patch.object(runtime.os,'open') as opened:
            for bad in (dict(origin,path='/home/fry/artifacts/foreign-origin.json'),
                        dict(origin,bytes=origin['bytes']+1),dict(origin,sha256='f'*64)):
                with self.assertRaisesRegex(ValueError,'exact retained guard origin'):
                    runtime.RuntimeAdmission({},_guard_origin=fixed._pin_tuple(bad))
            admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
            with self.assertRaisesRegex(ValueError,'exact retained guard origin'):
                admission._load(runtime.FIT_PROTECTED_COUNTER,4096,
                    _origin=(logical,('/home/fry/artifacts/foreign',734,'f'*64)))
            opened.assert_not_called()

    def test_origin_reader_releases_nested_owner_without_garbage_collection(self):
        from unittest import mock
        import gc,weakref
        import pa1_fixed40_runtime as runtime
        references=[]
        def load(owner,*args,**kwargs):
            references.append(weakref.ref(owner));return {}
        origin=dict(runtime.FIT_PROTECTED_COUNTER,path=str(
            Path(runtime.FIT_PROTECTED_COUNTER['path']).parent/'guard-origin.json'))
        enabled=gc.isenabled();gc.disable()
        try:
            with mock.patch.object(runtime.RuntimeAdmission,'_load',load):
                try:runtime.RuntimeAdmission({},_guard_origin=fixed._pin_tuple(origin))
                except ValueError:pass
            self.assertEqual(len(references),1)
            self.assertIsNone(references[0]())
        finally:
            if enabled:gc.enable()

    def test_runtime_counter_captures_review_anchor_before_public_mutation(self):
        from unittest import mock
        import pa1_fixed40_runtime as runtime
        pin=dict(path='/home/fry/artifacts/NONHISTORICAL-guard-anchor',bytes=0,sha256='d'*64)
        admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        admission._profile_wire=fixed.core.canonical(dict(development=pin,authority=pin,
            history=pin,events=[214,0,0],consumed=dict.fromkeys(runtime.RUNTIME_HARD,0)))
        admission.envelope=dict(counter_limits=dict(fixed.RecoveryCounters.LIMITS),
            event_limits=(215,4,4),development_rounds=2)
        admission.profile_pin=dict(pin,sha256='a'*64)
        admission.accepted=True;admission.review_sha='b'*64
        admission.event_record_hashes=();admission.read_trace=[];admission._graphs=[]
        counter_type=admission._counter_type()
        admission.accepted=False;admission.review_sha='e'*64
        admission.profile_pin['sha256']='f'*64
        counter=counter_type.__new__(counter_type)
        counter.trace_io=True;counter._io_trace=[{}]
        value=dict.fromkeys(runtime.RUNTIME_HARD,0)
        value.update(failures=[214,0,0],recovery=dict(inherited=[214,0,0],
            plan='a'*64,preflight='b'*64))
        # Exercise the added predicate after the unchanged full parent reader.
        with mock.patch.object(fixed.RecoveryCounters,'_read',return_value=value):
            self.assertIs(counter._read(),value)
            for field in ('plan','preflight'):
                original=value['recovery'][field]
                value['recovery'][field]='e'*64
                with self.assertRaisesRegex(ValueError,'immutable accepted runtime review anchor'):
                    counter._read()
                value['recovery'][field]=original


class TwentyPointQuotaBudgetTests(unittest.TestCase):
    """Current-window twenty-point authority retains its immutable anchor."""
    def fixture(self, drift=0, remaining=98.0):
        import pa1_fixed40_constructors as constructors
        budget=dict(baseline_remaining_percent=99.0,baseline_used_percent=1.0,
            consume_percent=20.0,floor_remaining_percent=79.0,
            observed_utc='2026-10-23T18:51:45.602000+00:00',resets_at=1793382070,
            schema_version=1,session_id='01a14b66-bb49-7e22-8a8c-1cb04c03816e')
        body=(json.dumps(budget,sort_keys=True)+'\n').encode()
        identity=hashlib.sha256(body).hexdigest()
        self.assertEqual(identity,'7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4')
        consumed=99.0-remaining
        value=dict(age_seconds=0,baseline_observed_utc=budget['observed_utc'],
            baseline_remaining_percent=99.0,baseline_resets_at=budget['resets_at'],
            baseline_session_id=budget['session_id'],baseline_used_percent=1.0,
            budget_file=constructors.QUOTA_BUDGET_IDENTITIES[identity],budget_initialized=False,
            consume_percent=20.0,consumed_percent=consumed,consumed_percentage_points=consumed,
            effort='medium',floor_remaining_percent=79.0,log='/home/fry/artifacts/fixture-log',
            model='gpt-6.1-sol',observed_utc=budget['observed_utc'],reason='fixture',
            remaining_percent=remaining,reset_drift_seconds=drift,reset_tolerance_seconds=10,
            resets_at=budget['resets_at']+drift,session_id=budget['session_id'],
            status='allow_start',used_percent=100.0-remaining)
        return constructors,body,value

    def test_twenty_point_floor_and_anchored_jitter(self):
        for drift in (-10,0,10):
            c,budget,value=self.fixture(drift,79.1);raw=json.dumps(value).encode()
            # Use the exact subtraction used by the telemetry producer.
            value['consumed_percent']=value['consumed_percentage_points']=99.0-79.1
            raw=json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw,maxima=c.ROLE_MAXIMA,budget_raw=budget,
                live_bytes=0,now_utc=value['observed_utc']),raw)
        for drift,remaining in ((-11,98.0),(11,98.0),(20,98.0),(0,79.0),(0,78.0)):
            with self.subTest(drift=drift,remaining=remaining):
                c,budget,value=self.fixture(drift,remaining)
                with self.assertRaises(ValueError):
                    c.quota_snapshot(json.dumps(value).encode(),maxima=c.ROLE_MAXIMA,
                        budget_raw=budget,live_bytes=0,now_utc=value['observed_utc'])

    def test_twenty_point_identity_rollback_expiry_and_baseline(self):
        from datetime import datetime,timezone
        for case in ('unknown_pin','rollback','expired','forged_drift','pooled_floor','other_session'):
            with self.subTest(case=case):
                c,budget,value=self.fixture()
                if case=='unknown_pin':budget=budget.replace(b'99.0',b'98.0',1)
                if case=='rollback':value.update(remaining_percent=100.0,used_percent=0.0,
                    consumed_percent=-1.0,consumed_percentage_points=-1.0)
                if case=='expired':value['observed_utc']=datetime.fromtimestamp(
                    value['baseline_resets_at'],timezone.utc).isoformat()
                if case=='forged_drift':value['reset_drift_seconds']=1
                if case=='pooled_floor':value['floor_remaining_percent']=69.0
                if case=='other_session':value['session_id']='other-session'
                with self.assertRaises(ValueError):
                    c.quota_snapshot(json.dumps(value).encode(),maxima=c.ROLE_MAXIMA,
                        budget_raw=budget,live_bytes=0,now_utc=value['observed_utc'])


class Round150QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""
    def fixture(self,remaining=88.0,drift=0):
        import pa1_fixed40_constructors as c
        budget={'baseline_remaining_percent': 89.0, 'baseline_used_percent': 11.0, 'consume_percent': 10.0, 'floor_remaining_percent': 79.0, 'observed_utc': '2026-10-23T23:44:12.139000+00:00', 'resets_at': 1793382070, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body=(json.dumps(budget,sort_keys=True)+'\n').encode()
        identity=hashlib.sha256(body).hexdigest()
        self.assertEqual(identity,'6c5931f6a92e48e3240a86035a7564a12ec3084995d36ca2bf41be429756bfc2')
        consumed=budget['baseline_remaining_percent']-remaining
        value=dict(age_seconds=0,baseline_observed_utc=budget['observed_utc'],
            baseline_remaining_percent=budget['baseline_remaining_percent'],
            baseline_used_percent=budget['baseline_used_percent'],baseline_resets_at=budget['resets_at'],
            baseline_session_id=budget['session_id'],session_id=budget['session_id'],
            budget_file=c.QUOTA_BUDGET_IDENTITIES[identity],budget_initialized=False,
            consume_percent=10.0,consumed_percent=consumed,consumed_percentage_points=consumed,
            effort='medium',floor_remaining_percent=79.0,log='/home/fry/artifacts/NONHISTORICAL-telemetry',
            model='gpt-6.1-sol',observed_utc=budget['observed_utc'],reason='NONHISTORICAL fixture',
            remaining_percent=remaining,used_percent=100.0-remaining,
            reset_drift_seconds=drift,reset_tolerance_seconds=10,resets_at=budget['resets_at']+drift,
            status='allow_start')
        return c,body,value

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10,0,10):
            c,budget,value=self.fixture(79.1,drift);raw=json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw,maxima=c.ROLE_MAXIMA,budget_raw=budget,
                live_bytes=0,now_utc=value['observed_utc']),raw)
        for remaining,drift in ((79.0,0),(78.0,0),(88.0,-11),(88.0,11)):
            c,budget,value=self.fixture(remaining,drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(),maxima=c.ROLE_MAXIMA,
                    budget_raw=budget,live_bytes=0,now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline','pooled_floor','old_twenty_points','rollback','forged_drift','unknown_budget'):
            c,budget,value=self.fixture()
            if case=='pooled_baseline':value['baseline_remaining_percent']=99.0
            if case=='pooled_floor':value['floor_remaining_percent']=69.0
            if case=='old_twenty_points':value['consume_percent']=20.0
            if case=='rollback':value.update(remaining_percent=90.0,used_percent=10.0,
                consumed_percent=-1.0,consumed_percentage_points=-1.0)
            if case=='forged_drift':value['reset_drift_seconds']=1
            if case=='unknown_budget':budget=budget.replace(b'89.0',b'88.0',1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(),maxima=c.ROLE_MAXIMA,
                    budget_raw=budget,live_bytes=0,now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime,timezone
        c,budget,value=self.fixture()
        value['observed_utc']=datetime.fromtimestamp(value['baseline_resets_at'],timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(),maxima=c.ROLE_MAXIMA,
                budget_raw=budget,live_bytes=0,now_utc=value['observed_utc'])
        old='7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old],
            '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old],277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old],10)


class QuotaBudgetJitterTests(unittest.TestCase):
    """Bind role80 to the saved current budget, never a moving reset anchor."""
    def fixture(self,drift=0):
        import pa1_fixed40_constructors as constructors
        budget=dict(baseline_remaining_percent=92.0,baseline_used_percent=8.0,
            consume_percent=10.0,floor_remaining_percent=82.0,
            observed_utc='2026-10-23T16:57:25.049000+00:00',resets_at=1793316301,
            schema_version=1,session_id='01a14b66-bb49-7e22-8a8c-1cb04c03816e')
        body=(json.dumps(budget,sort_keys=True)+'\n').encode()
        identity=hashlib.sha256(body).hexdigest()
        self.assertEqual(identity,'a78ace94ec73a8c7e5bfcf4092d9baa0adcbac95101ed28921d553b17ff90fbc')
        value=dict(age_seconds=0,baseline_observed_utc=budget['observed_utc'],
            baseline_remaining_percent=92.0,baseline_resets_at=budget['resets_at'],
            baseline_session_id=budget['session_id'],baseline_used_percent=8.0,
            budget_file=constructors.QUOTA_BUDGET_IDENTITIES[identity],budget_initialized=False,
            consume_percent=10.0,consumed_percent=1.0,consumed_percentage_points=1.0,
            effort='medium',floor_remaining_percent=82.0,log='/home/fry/artifacts/fixture-log',
            model='gpt-6.1-sol',observed_utc=budget['observed_utc'],reason='fixture',
            remaining_percent=91.0,reset_drift_seconds=drift,reset_tolerance_seconds=10,
            resets_at=budget['resets_at']+drift,session_id=budget['session_id'],
            status='allow_start',used_percent=9.0)
        return constructors,body,value

    def test_current_budget_accepts_only_anchored_ten_second_jitter(self):
        for drift in (-10,0,10):
            with self.subTest(drift=drift):
                c,budget,value=self.fixture(drift);raw=json.dumps(value).encode()
                self.assertEqual(c.quota_snapshot(raw,maxima=c.ROLE_MAXIMA,budget_raw=budget,
                    live_bytes=0,now_utc=value['observed_utc']),raw)
        # Prior successful+10 must not become the anchor for a second+10.
        for drift in (-11,11,20):
            c,budget,value=self.fixture(drift)
            with self.assertRaisesRegex(ValueError,'anchored'):
                c.quota_snapshot(json.dumps(value).encode(),maxima=c.ROLE_MAXIMA,
                    budget_raw=budget,live_bytes=0,now_utc=value['observed_utc'])

    def test_current_budget_rejects_rollback_expiry_and_forged_drift(self):
        from datetime import datetime,timezone
        for case in ('rollback','expired','forged_drift','other_session'):
            with self.subTest(case=case):
                c,budget,value=self.fixture()
                if case=='rollback':value.update(remaining_percent=93.0,used_percent=7.0,
                    consumed_percent=-1.0,consumed_percentage_points=-1.0)
                if case=='expired':value['observed_utc']=datetime.fromtimestamp(
                    value['baseline_resets_at'],timezone.utc).isoformat()
                if case=='forged_drift':value['reset_drift_seconds']=1
                if case=='other_session':value['session_id']='other-session'
                with self.assertRaises(ValueError):
                    c.quota_snapshot(json.dumps(value).encode(),maxima=c.ROLE_MAXIMA,
                        budget_raw=budget,live_bytes=0,now_utc=value['observed_utc'])


class CompleteDataflowProgramTests(unittest.TestCase):
    """Exercise the real future-program checker without native or counter IO."""

    def test_writer_rejects_allocation_before_destination_creation(self):
        cases=('caller','initial_owner','first_section','growing_owner')
        with tempfile.TemporaryDirectory() as directory:
            for case in cases:
                with self.subTest(case=case):
                    path=Path(directory)/(case+'.jsonl');visited=[]
                    value=({'rows':[{'payload':i} for i in range(1500)]}
                           if case=='first_section' else {})
                    def sections():
                        visited.append(True)
                        yield 'header',value
                    def owner():
                        return fixed.core.SEMANTIC if (case=='initial_owner' or
                            case=='growing_owner' and visited) else 0
                    caller=(fixed.core.SEMANTIC if case=='caller' else
                        fixed.core.SEMANTIC-2*fixed.core.CHUNK-262144-65536
                        if case=='first_section' else 0)
                    with self.assertRaisesRegex(ValueError,'lifetime|allocation'):
                        fixed.write_complete_fit_representation(path,sections(),
                            live_bytes=caller,owner_charge=owner)
                    self.assertFalse(path.exists(),'allocation rejection created destination')
                    self.assertEqual(len(visited),0 if case in ('caller','initial_owner') else 1)

    def specimen(self, *, parameters=None, constructor_rows=None):
        controls={};cases=[]
        for role in range(81):
            body=fixed.core.canonical(dict(schema='fixed40-constructor-arguments-v1',
                role=role,parameters=parameters or {},byte_inputs={},json_inputs={},
                wire_inputs={},declaration_bindings={}))
            pin=dict(path='/home/fry/artifacts/NONHISTORICAL-arguments-%d'%role,
                bytes=len(body),sha256=hashlib.sha256(body).hexdigest())
            controls[pin['path']]=body
            output=dict(path='/home/fry/artifacts/NONHISTORICAL-output-%d'%role,
                        bytes=1,sha256=hashlib.sha256(b'x').hexdigest())
            cases.append(dict(role=role,arguments=pin,output=output,
                constructor_rows=constructor_rows or [],dependencies=[]))
        # Real forward dataflow families: logs before gates, candidate before
        # its review, native preflight before binary mapping and freeze.
        for role,parents in ((0,[1]),(34,[54,38]),(41,[42]),(44,[55]),
                             (56,[54,55]),(57,[56]),(59,[57]),(61,[57,59]),
                             (60,[59,61]),(62,[57,59,60])):
            cases[role]['dependencies']=parents
        return cases,controls

    def build(self,cases,controls,archive_wires=()):
        import pa1_fixed40_constructors as constructors
        return fixed.build_complete_fit_program(cases,
            tuple('archive:'+str(i) for i in range(len(archive_wires))),archive_wires,
            constructors.ROLE_MAXIMA,(),read_control=lambda pin:controls[pin['path']],
            live_bytes=fixed.graph_charge(controls))

    def test_full81_243_program_preserves_forward_dependencies(self):
        cases,controls=self.specimen()
        future,pool=self.build(cases,controls)
        self.assertEqual(len(future),81);self.assertEqual(len(pool),243)
        positions={row['role']:i for i,row in enumerate(future)}
        self.assertLess(positions[54],positions[34])
        self.assertLess(positions[61],positions[60])
        self.assertLess(positions[42],positions[41])
        for index,row in enumerate(future):
            self.assertEqual(row['units'],[index*3,index*3+1,index*3+2])
            producer=json.loads(pool[index*3+1]);consumer=json.loads(pool[index*3+2])
            self.assertEqual(consumer[:-1],producer)
            self.assertEqual(consumer[-1][1][0],row['output']['path'])

    def test_duplicate_body_occurrences_and_archive_ids_remain_distinct(self):
        pin=dict(path='/home/fry/artifacts/NONHISTORICAL-body',bytes=3,sha256='a'*64)
        cases,controls=self.specimen(constructor_rows=[(pin,'evidence','bytes')]*2)
        archive=(fixed.core.canonical([[617,[pin['path'],3,'a'*64],'evidence','bytes']]),)
        future,pool=self.build(cases,controls,archive)
        producer=json.loads(pool[1])
        self.assertEqual(producer[0][0],0)
        self.assertNotEqual(producer[1][0],producer[2][0])
        self.assertEqual(producer[1][1:],producer[2][1:])
        self.assertNotIn(617,[row[0] for wire in pool for row in json.loads(wire)])

    def test_sparse_archive_and_full101_paired_sources_fit_original_occurrence_cap(self):
        import pa1_fixed40_constructors as constructors
        cases,controls=self.specimen()
        sources=[dict(path='/home/fry/artifacts/NONHISTORICAL-source-%03d'%i,
                      bytes=1,sha256=hashlib.sha256(str(i).encode()).hexdigest())
                 for i in range(101)]
        source_wire=fixed.core.canonical([[p,'source','bytes'] for p in sources])
        source_rows_pin=dict(path='/home/fry/artifacts/NONHISTORICAL-full101-rows',
            bytes=len(source_wire),sha256=hashlib.sha256(source_wire).hexdigest())
        controls[source_rows_pin['path']]=source_wire
        for role in (29,30,60,79):
            cases[role]['constructor_rows']=dict(source_rows_pin)
        old_ids=list(range(1056))+[1568]
        p=dict(path='/home/fry/artifacts/NONHISTORICAL-old-body',bytes=1,sha256='a'*64)
        rows=[[i,[p['path'],1,p['sha256']],'evidence','bytes'] for i in old_ids]
        # The real archive has676units and1057 declarations with a sparse
        # high watermark1568. Keep these dimensions without a giant JSON unit.
        groups=[rows[i*2:i*2+2] for i in range(381)]
        groups.extend([row] for row in rows[762:])
        archive=tuple(fixed.core.canonical(group) for group in groups)
        original=set(old_ids);del rows,groups,p,old_ids
        future,pool=fixed.build_complete_fit_program(cases,
            tuple('archive:'+str(i) for i in range(676)),archive,
            constructors.ROLE_MAXIMA,sources,read_control=lambda p:controls[p['path']],
            live_bytes=fixed.graph_charge(controls))
        by_role={row['role']:row for row in future}
        def source_rows(role):
            return [r for r in json.loads(pool[by_role[role]['units'][1]]) if r[2]=='source']
        self.assertEqual(source_rows(29),source_rows(30))
        self.assertEqual(source_rows(60),source_rows(79))
        self.assertNotEqual(source_rows(29),source_rows(60))
        uses={}
        for wire in pool:
            for occurrence,pin,kind,mode in json.loads(wire):
                self.assertNotIn(occurrence,original)
                uses[occurrence]=uses.get(occurrence,0)+(2 if mode=='json' else 1)
        self.assertLessEqual(max(uses),2047)
        self.assertLessEqual(max(uses.values()),8)
        self.assertEqual(len(source_rows(60)),101)
        self.assertEqual(sum(r[2]=='source' for wire in pool for r in json.loads(wire)),808)

    def test_paired_source_reuse_rejects_duplicate_within_one_closure(self):
        import pa1_fixed40_constructors as constructors
        cases,controls=self.specimen()
        p=dict(path='/home/fry/artifacts/NONHISTORICAL-paired-source',bytes=1,sha256='a'*64)
        cases[60]['constructor_rows']=[[p,'source','bytes']]*2
        with self.assertRaisesRegex(ValueError,'duplicate source in paired closure'):
            fixed.build_complete_fit_program(cases,(),(),constructors.ROLE_MAXIMA,[p],
                read_control=lambda p:controls[p['path']],live_bytes=fixed.graph_charge(controls))

    def test_encoded_consumer_prefix_comparison_rejects_changed_full_input(self):
        import pa1_fixed40_constructors as constructors
        p=dict(path='/home/fry/artifacts/NONHISTORICAL-prefix-body',bytes=1,sha256='a'*64)
        cases,controls=self.specimen(constructor_rows=[[p,'evidence','bytes']])
        future,pool=self.build(cases,controls)
        changed=list(pool);consumer=json.loads(changed[2])
        consumer[1][1][2]='b'*64
        changed[2]=fixed.core.canonical(consumer)
        with self.assertRaisesRegex(ValueError,'identical full input schedule'):
            fixed._fit_check_program(future,tuple(changed),(),(),constructors.ROLE_MAXIMA,
                                     live_bytes=fixed.graph_charge((cases,controls)))

    def test_cycle_and_duplicate_roles_fail_before_control_reads(self):
        cases,_=self.specimen();cases[1]['dependencies']=[0]
        with self.assertRaisesRegex(ValueError,'dependency cycle'):
            fixed.complete_fit_role_order(cases)
        cases,_=self.specimen();cases[80]['role']=79
        with self.assertRaisesRegex(ValueError,'unique complete'):
            fixed.complete_fit_role_order(cases)

    def test_original_child_alias_occurrences_cannot_be_renumbered_or_retargeted(self):
        pin=dict(path='/home/fry/artifacts/NONHISTORICAL-original-child',bytes=3,sha256='a'*64)
        original=[524,[pin['path'],3,'a'*64],'evidence','bytes']
        cases,controls=self.specimen();cases[28]['constructor_rows']=[original]
        archive=(fixed.core.canonical([original]),)
        future,pool=self.build(cases,controls,archive)
        step=next(row for row in future if row['role']==28)
        self.assertEqual(json.loads(pool[step['units'][1]])[-1],original)
        cases[28]['constructor_rows']=[[524,[pin['path'],4,'b'*64],'evidence','bytes']]
        with self.assertRaisesRegex(ValueError,'original child declaration equality'):
            self.build(cases,controls,archive)

    def test_pinned_rows_are_fully_read_once_per_role_and_tamper_fails(self):
        pin=dict(path='/home/fry/artifacts/NONHISTORICAL-row-body',bytes=3,sha256='a'*64)
        rows=fixed.core.canonical([[pin,'evidence','bytes'],[pin,'evidence','bytes']])
        row_pin=dict(path='/home/fry/artifacts/NONHISTORICAL-complete-rows',
            bytes=len(rows),sha256=hashlib.sha256(rows).hexdigest())
        cases,controls=self.specimen()
        for case in cases:case['constructor_rows']=dict(row_pin)
        controls[row_pin['path']]=rows
        import pa1_fixed40_constructors as constructors
        observed=[]
        def reader(pin):
            observed.append(pin['path']);return controls[pin['path']]
        future,pool=fixed.build_complete_fit_program(cases,(),(),constructors.ROLE_MAXIMA,
            (),read_control=reader,live_bytes=fixed.graph_charge(controls))
        self.assertEqual(observed.count(row_pin['path']),81)
        self.assertEqual(len(pool),243)
        controls[row_pin['path']]=b'[]\n'
        with self.assertRaisesRegex(ValueError,'rows authentication'):
            fixed.build_complete_fit_program(cases,(),(),constructors.ROLE_MAXIMA,(),
                read_control=reader,live_bytes=fixed.graph_charge(controls))

    def test_tampered_control_and_unresolved_helpers_fail(self):
        cases,controls=self.specimen();controls[cases[1]['arguments']['path']]=b'{}'
        with self.assertRaisesRegex(ValueError,'authentication'):self.build(cases,controls)
        cases,controls=self.specimen(parameters=dict(native=dict(
            kind='unresolved-current-executable-identity')))
        with self.assertRaisesRegex(ValueError,'unresolved complete'):self.build(cases,controls)
        # A real strict unavailable-history value is semantic data, not a
        # recipe helper. Preserve it rather than hiding unavailable history.
        fixed._fit_require_resolved_arguments(dict(kind='successful_seek_without_matching_version'))

    def test_complete_controls_and_writer_preserve_topological_program(self):
        import pa1_fixed40_constructors as constructors
        cases,controls=self.specimen();future,pool=self.build(cases,controls)
        def pin(name):
            return dict(path='/home/fry/artifacts/NONHISTORICAL-'+name,bytes=1,sha256='a'*64)
        sources=[pin('source-'+str(i)) for i in range(90)]
        tools=[pin('tool-'+str(i)) for i in range(11)]
        profile=pin('profile')
        compiled=dict(maxima=constructors.ROLE_MAXIMA,consumed_events=(214,0,0),
            inherited_event_history=[],prior83_wire=fixed.core.canonical([pin('prior-'+str(i)) for i in range(83)]),
            profile_binding=dict(profile=profile,authority=pin('authority'),history=pin('history'),
                events=[214,0,0],source_identity=fixed.core.digest(sources+tools)))
        owned=dict(compiled=compiled,inputs=dict(source_freeze=pin('source-freeze'),
            tool_freeze=pin('tool-freeze')),sources=sources,tools=tools,historical_scope={},
            future=future,future_pool=pool,archive_ids=(),archive_wires=(),
            matrix_storage=dict(schema='fixed40-matrix-storage-binding-v1',
                activation=dict(fixed.MATRIX_STORAGE_ACTIVATION),profile=profile,
                contexts_digest='a'*64,directory='/home/fry/artifacts/NONHISTORICAL-matrix',
                ledger='/home/fry/artifacts/NONHISTORICAL-ledger',round=0,phase='normal'))
        del compiled,future,pool,sources,tools,cases,controls,profile
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'complete.jsonl';observed=[]
            def reserve(packet):
                self.assertFalse(path.exists());observed.extend(packet)
            result=fixed.assemble_complete_fit_representation(path,owned,
                publication_charge=reserve,live_bytes=32768)
            body=path.read_bytes()
        self.assertEqual(owned,{})
        self.assertEqual(observed[0]['bytes'],len(body))
        self.assertEqual(result['pin']['sha256'],hashlib.sha256(body).hexdigest())
        sections=[json.loads(line) for line in body.splitlines()]
        self.assertEqual([row[0] for row in sections],list(fixed.COMPLETE_FIT_SECTIONS))
        values=dict(sections);order=[row['role'] for row in values['future81']]
        self.assertLess(order.index(54),order.index(34))
        self.assertEqual([row['role'] for row in values['controls']['future_arguments']],order)
        self.assertFalse(result['accepted'])

    def test_full_external_lifetime_fails_before_open(self):
        cases,_=self.specimen()
        with mock.patch('builtins.open',side_effect=AssertionError('unadmitted read')):
            with self.assertRaisesRegex(ValueError,'lifetime|descriptors'):
                fixed.build_complete_fit_program(cases,(),(),[1]*81,(),
                    read_control=lambda pin:(_ for _ in ()).throw(AssertionError('read')),
                    live_bytes=fixed.core.SEMANTIC)


class Selected40Tests(unittest.TestCase):
    def test_archive_context_transfer_preserves_every_field_and_instruction(self):
        context=dict(reference_placements=[{'references':['full-child']}]*3,
            historical_equivalence_parse={'output_bytes':7},correction_record48={'failures':[]},
            source28_prefix='complete findings\nUTF8 é',scope36_pin={'path':'fixture'},
            retention_amendment={'manifest':['complete original history']})
        schedule=dict(stages=(('prepare/validate',(),tuple(range(29))),),accepted=False)
        compiled=dict(context=context,schedule=schedule)
        sources=[dict(path='/home/fry/artifacts/NONHISTORICAL-source-'+str(i),bytes=i,sha256='a'*64)
            for i in range(90)]
        tools=[dict(path='/home/fry/artifacts/NONHISTORICAL-tool-'+str(i),bytes=i,sha256='b'*64)
            for i in range(11)]
        instructions=(('input','source_freeze'),('record',28),('scope36',None))
        before=fixed.core.canonical((compiled,instructions,sources,tools))
        storage=fixed.archive_context_wires(compiled,instructions,sources,tools,live_bytes=0)
        live=fixed.graph_charge(storage)+524288
        for key,value in context.items():
            self.assertEqual(fixed.archive_context_value(storage,key,live_bytes=live),value)
        for index,row in enumerate(instructions):
            self.assertEqual(fixed.archive_context_value(storage,'instruction',index=index,live_bytes=live),list(row))
        for key,value in (('sources',sources),('tools',tools),('schedule',schedule)):
            self.assertEqual(fixed.archive_context_value(storage,key,live_bytes=live),
                json.loads(fixed.core.canonical(value)))
        self.assertEqual(fixed.core.canonical((compiled,instructions,sources,tools)),before)
        self.assertLess(fixed.graph_charge(storage),fixed.graph_charge((context,instructions,sources,tools,schedule)))
        with mock.patch.object(fixed.records,'decode',side_effect=AssertionError('allocated')):
            with self.assertRaisesRegex(ValueError,'predecode allocation'):
                fixed.archive_context_value(storage,'sources',live_bytes=fixed.core.SEMANTIC)

    def test_projected_historical_plan_preserves_complete_original_contract(self):
        original = fixed.historical_plan()
        projected = fixed.historical_plan_wires(live_bytes=524288)
        self.assertEqual(projected['units'], tuple(fixed.core.canonical(r) for r in original['declarations']))
        self.assertEqual(projected['unit_ids'], tuple(original['unit_ids']))
        self.assertEqual(projected['logical_aliases'], tuple(original['logical_aliases']))
        self.assertEqual(projected['future_role_caps'], tuple(original['future_role_caps']))
        self.assertEqual(projected['allocation']['original_full_graph'], fixed.graph_charge(original))
        self.assertLessEqual(projected['allocation']['peak'], fixed.core.SEMANTIC)
        self.assertEqual(projected['allocation']['caller'], 524288)
        with mock.patch.object(fixed, '_read') as read:
            with self.assertRaisesRegex(ValueError, 'allocation'):
                fixed.historical_plan_wires(live_bytes=fixed.core.SEMANTIC)
            read.assert_not_called()

    def test_all_prior83_bindings_match_full_original_mapping_and_units(self):
        projected=fixed.historical_plan_wires()
        bindings=fixed.prior83_bindings(projected['units'],
            live_bytes=fixed.graph_charge(projected))
        self.assertEqual(tuple(x['occurrence'] for x in bindings),tuple(range(588,671)))
        self.assertEqual(sum(x['kind']=='source' for x in bindings),81)
        self.assertEqual(sum(x['pin']['bytes'] for x in bindings[-2:]),400058000)
        self.assertTrue(all(x['parse_rule'] and x['authority_ref'] for x in bindings))
        changed=list(projected['units'])
        for index,raw in enumerate(changed):
            rows=json.loads(raw)
            if any(row[0]==588 for row in rows):
                for row in rows:
                    if row[0]==588:row[1][2]='a'*64
                changed[index]=fixed.core.canonical(rows)
                break
        with self.assertRaisesRegex(ValueError,'exact prior83 original binding'):
            fixed.prior83_bindings(tuple(changed))
        with mock.patch.object(fixed.os,'open') as opened:
            with self.assertRaisesRegex(ValueError,'projection allocation'):
                fixed.prior83_bindings(projected['units'],live_bytes=fixed.core.SEMANTIC)
            opened.assert_not_called()

    def test_projection_keeps_global_shape_and_duplicate_key_rejections(self):
        raw = fixed._read(fixed.PLAN)
        duplicate = raw.replace(b'"unit_count":380', b'"unit_count":380,"unit_count":380')
        self.assertNotEqual(raw, duplicate)
        oversized = json.loads(raw)
        oversized['scope'] = [0]*fixed.records.NODE_CAP
        deep = json.loads(raw)
        deep['scope'] = [[[[[[[[[[0]]]]]]]]]]
        for bad in (duplicate, fixed.core.canonical(oversized), fixed.core.canonical(deep)):
            with self.subTest(bytes=len(bad)), mock.patch.object(fixed, '_read', return_value=bad):
                with self.assertRaises(ValueError):
                    fixed.historical_plan_wires()

    def test_projection_rejects_omitted_and_changed_occurrences(self):
        original = fixed.historical_plan()
        for changed in ('missing', 'duplicate'):
            value = json.loads(fixed.core.canonical(original))
            if changed == 'missing': value['declarations'].pop()
            else: value['declarations'][-1][0][0] = value['declarations'][0][0][0]
            with self.subTest(changed=changed), mock.patch.object(
                    fixed, '_read', return_value=fixed.core.canonical(value)):
                with self.assertRaises(ValueError):
                    fixed.historical_plan_wires()

    def test_exact_native_first_d_inline_shape_fits_without_overlapping_graphs(self):
        """Use both full bodies from the retained failed attempt, at its tight boundary."""
        root = fixed.ROOT
        value = json.loads((root/'attempt2-native-comparison.json').read_bytes())
        # Keep the complete retained bodies; explicitly add their unchanged Cargo identity.
        for pair, field in zip(value['pairs'], ('/source_profile/compiled/8', '/authority/sources/14')):
            a, ak = fixed._location(json.loads(pair['before_body_utf8']), field)
            b, bk = fixed._location(json.loads(pair['after_body_utf8']), field)
            pair['mappings'].append(dict(field=field, predecessor=a[ak], successor=b[bk]))
        sources = json.loads((root/'attempt2-source-freeze.json').read_bytes())
        tools = json.loads((root/'attempt2-tool-freeze.json').read_bytes())
        executable = json.loads(value['pairs'][0]['after_body_utf8'])['source_profile']['executable']
        live = fixed.graph_charge(value)
        raw_reports = [pair[key].encode() for pair in value['pairs']
                       for key in ('before_body_utf8', 'after_body_utf8')]
        peak = max(2*len(raw)+128*fixed.core._wire_nodes(raw) for raw in raw_reports)
        metadata = (fixed.core.SEMANTIC-live-peak-fixed.core.CHUNK-262144
                    -fixed.graph_charge('a'*64))
        self.assertGreater(metadata, 0)

        def run(body, extra=0):
            owner = SimpleNamespace(operations=bytearray(2048), metadata_charge=lambda: metadata+extra)
            unit = SimpleNamespace(invocation=owner, declaration_charge=0)
            fixed._native_payload(body, sources, tools, executable, unit, 686, live,
                                  tuple(pair['after'] for pair in body['pairs']))
            self.assertEqual(owner.operations[686], 4)

        run(value)
        with self.assertRaisesRegex(ValueError, 'inline report preparse allocation'):
            run(value, 1)
        changed = json.loads(json.dumps(value))
        changed['pairs'][1]['before_body_utf8'] += ' '
        with self.assertRaisesRegex(ValueError, 'retained full report pin'):
            run(changed)
        changed = json.loads(json.dumps(value))
        changed['pairs'][0]['mappings'][0]['successor']['sha256'] = 'a'*64
        with self.assertRaises(ValueError):
            run(changed)
        changed = json.loads(json.dumps(value))
        pair = changed['pairs'][0]
        pair['after_body_utf8'] = pair['after_body_utf8'].replace(
            'all_96_authenticated_native_totals', 'bad_96_authenticated_native_totals')
        raw = pair['after_body_utf8'].encode()
        pair['after']['bytes'] = len(raw)
        pair['after']['sha256'] = hashlib.sha256(raw).hexdigest()
        with self.assertRaisesRegex(ValueError, 'complete current report inequality'):
            run(changed)

    def test_shared_cargo_successor_rejects_forged_identity_and_position(self):
        cargo_path = fixed.REPO/'experiments/evm_feasibility/snapshot_qualifier/Cargo.toml'
        raw = cargo_path.read_bytes()
        source = dict(path=str(cargo_path), bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest())
        original = dict(path='../Cargo.toml', bytes=901,
                        sha256='63b643eca00b29108eef0b70fdcdfddb278dc6a22a08ab295f84c6e4a49304cd')
        executable = dict(path='/home/fry/artifacts/frozen-comparator', bytes=1, sha256='a'*64)
        rows = [dict(path='fixed-'+str(i), bytes=1, sha256='b'*64) for i in range(14)]
        before = dict(vrf_sources=[], authority=dict(six_input_owner=dict(executable=executable,
                                                                       sources=rows+[original])))
        after = json.loads(json.dumps(before))
        after['authority']['six_input_owner']['sources'][14] = dict(source, path='../Cargo.toml')
        fixed.verify_shared_pair(json.loads(json.dumps(before)), after, 0, [source], executable)
        for field, value in [('path', '../other.toml'), ('bytes', 0), ('sha256', '0'*64)]:
            bad = json.loads(json.dumps(after))
            bad['authority']['six_input_owner']['sources'][14][field] = value
            with self.assertRaises(ValueError):
                fixed.verify_shared_pair(json.loads(json.dumps(before)), bad, 0, [source], executable)
        misplaced = json.loads(json.dumps(before))
        misplaced['authority']['six_input_owner']['sources'].reverse()
        with self.assertRaisesRegex(ValueError, 'Cargo provenance position'):
            fixed.verify_shared_pair(misplaced, json.loads(json.dumps(misplaced)), 0, [source], executable)

    def test_fixed_mapping_projection_preserves_tuple_order_and_maxima(self):
        value = fixed.selected_descriptor()
        self.assertEqual(len(value['slot']['rows']), 40)
        self.assertEqual(value['slot']['rows'][0]['parent_manifest_index'], 110)
        self.assertEqual(value['slot']['rows'][-1]['parent_manifest_index'], 2735)
        self.assertEqual(sum(value['maxima']), 10992546)
        self.assertFalse(value['complete_fit'])
        self.assertFalse(value['ready'])

    def test_descriptor_lifetime_rejects_before_io_and_preserves_global_roles(self):
        with mock.patch.object(fixed,'_read') as opened:
            with self.assertRaisesRegex(ValueError,'descriptor initial allocation'):
                fixed.selected_descriptor(live_bytes=fixed.core.SEMANTIC)
            opened.assert_not_called()
        descriptor=fixed.selected_descriptor(live_bytes=524288)
        self.assertEqual(len(descriptor['identities']),81)
        self.assertEqual(sum(descriptor['maxima']),10992546)
        raw=fixed._read(fixed.MAPPING)
        a,b=fixed._field(raw,'future_role_templates')
        duplicate=raw[:a]+raw[a:b].replace(b'"group":',b'"group":"current54","group":',1)+raw[b:]
        with mock.patch.object(fixed,'_read',return_value=duplicate):
            with self.assertRaises(ValueError):fixed.selected_descriptor()
        # The original array's GLOBAL node cap cannot reset for each row.
        rows=json.loads(raw[a:b])
        for row in rows:row['extra']=[0]*110
        oversized=raw[:a]+fixed.core.canonical(rows).rstrip()+raw[b:]
        with mock.patch.object(fixed,'_read',return_value=oversized):
            with self.assertRaisesRegex(ValueError,'global node cap|byte/shape cap'):
                fixed.selected_descriptor()

    def test_historical_preparation_preserves_all_occurrences_and_aliases(self):
        plan = fixed.historical_plan()
        self.assertEqual(plan['unit_count'], 380)
        self.assertEqual(len(plan['history_parts']), 11)
        self.assertEqual([a['occurrence'] for a in plan['logical_aliases']],
                         [572, 573, 574, 575, 576])

    def test_byte_framing_success_cannot_publish_complete_fit(self):
        plan = fixed.compile_selected40(consumed_events=(4, 0, 0),
                                        event_history=fixed.BASELINE_HISTORY)
        self.assertFalse(plan['complete_fit'])
        self.assertFalse(plan['accepted'])
        self.assertFalse(plan['ready'])
        self.assertEqual(len(plan['units']), 380)
        self.assertEqual(len(json.loads(plan['prior83_wire'])),83)
        self.assertEqual(plan['schedule']['retained_roles'], tuple(range(81)))
        with self.assertRaisesRegex(fixed.IncompleteFit, 'actual source/tool freezes'):
            fixed.semantic_units(plan, {})

    def test_s3_guard_preserves_four_and_rejects_eight_before_input_reads(self):
        from unittest.mock import patch
        fixed.require_live40_budget((4, 0, 0), fixed.BASELINE_HISTORY)
        with patch.object(fixed, '_read', side_effect=AssertionError('read after exhaustion')):
            with self.assertRaisesRegex(fixed.IncompleteFit, 'events exhausted'):
                fixed.compile_selected40(consumed_events=(8, 0, 0), event_history=fixed.BASELINE_HISTORY)
        for counts, history in (((9, 0, 0), fixed.BASELINE_HISTORY),
                                ((3, 0, 0), fixed.BASELINE_HISTORY),
                                ((4, 0), fixed.BASELINE_HISTORY),
                                ((True, 0, 0), fixed.BASELINE_HISTORY),
                                ((4, 0, 0), 'a'*64)):
            with self.assertRaises(ValueError):
                fixed.require_live40_budget(counts, history)

    def test_same_size_consumed_mutation_and_nonexact_eof_reject(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'body.json'
            original = b'{"a":1}'
            pin = dict(path=str(path), bytes=len(original),
                       sha256=hashlib.sha256(original).hexdigest())
            for body in (b'{"a":2}', original[:-1], original+b' '):
                path.write_bytes(body)
                with self.assertRaises(ValueError):
                    fixed._read(pin)
            path.write_bytes(original)
            self.assertEqual(fixed._read(pin), original)

    def test_stage_arithmetic_never_releases_or_discounts_pending_roles(self):
        maxima = fixed.selected_descriptor()['maxima']
        # This tests prospective arithmetic only, with no semantic release claim.
        stages = ((fixed.PHASES[0], (), tuple(range(29))),
                  (fixed.PHASES[0], (), tuple(range(29, 57))),
                  (fixed.PHASES[1], (), tuple(range(57, 63))),
                  (fixed.PHASES[2], (), tuple(range(63, 70))),
                  (fixed.PHASES[3], (), tuple(range(70, 81))))
        result = fixed.all_phase_reservations(maxima, (None,)*81, stages)
        self.assertEqual(len(result), 5)
        for stage in result:
            self.assertEqual(stage['lifecycle_reserved_bytes'], 10992546)
            self.assertEqual(stage['pending_roles'], 81)
            self.assertFalse(stage['accepted'])
        with self.assertRaisesRegex(ValueError, 'live evidence overflow'):
            fixed.all_phase_reservations(maxima, (None,)*81,
                ((fixed.PHASES[0], (), tuple(range(57))),)+stages[2:])

    def test_stage_duplicate_omission_and_wrong_phase_reject(self):
        maxima = (16384,)*81
        stages = tuple((phase, (), tuple(range(*bounds)))
                       for phase, bounds in zip(fixed.PHASES, fixed.PHASE_RANGES))
        for bad in (stages[:-1], stages+stages[-1:],
                    ((fixed.PHASES[1], (), tuple(range(57))),)+stages[1:]):
            with self.assertRaises(ValueError):
                fixed.all_phase_reservations(maxima, (None,)*81, bad)

    def test_sealed_stage_edges_keep_authority_receipt_and_full_lifecycle(self):
        maxima = fixed.selected_descriptor()['maxima']
        proof = fixed.sealed_stages(maxima, (None,)*81)
        self.assertEqual(proof['releases'][0]['after'], 28)
        self.assertEqual(proof['releases'][0]['roles'], tuple(range(28)))
        self.assertEqual(proof['retained_roles'], tuple(range(81)))
        self.assertEqual(sum(count for _, count in proof['reauthentication_routes']), 16)
        for row in proof['reservations']:
            self.assertEqual(row['lifecycle_reserved_bytes'], 10992546)
            self.assertLessEqual(row['live_bytes']+row['receipt_reserved_bytes'], fixed.core.LIVE)
        self.assertTrue(proof['conditional_on_publication'])
        self.assertFalse(proof['accepted'])

    def test_original64_children_and_wrapper_findings_do_not_become_current_gates(self):
        from unittest.mock import patch
        descriptor = fixed.selected_descriptor()
        context = fixed.reference_context(descriptor['source_rosters']['accepted_roles'])
        placements = context['reference_placements']
        self.assertEqual(sum(len(p['references'])+1 for p in placements), 64)
        for role, placement in zip((28, 37, 49), placements):
            value = fixed.wrapper_specimen(placement, str(fixed.ROOT/('current54-'+str(role)+'.json')))
            self.assertFalse(value['accepted'])
            self.assertEqual(len(value['findings'].encode()), 16384)
            self.assertEqual(value['references'], placement['references'])
            value['references'] = value['references'][:-1]
            with patch.object(fixed.records, 'authenticate', side_effect=AssertionError('coverage failed late')):
                with self.assertRaisesRegex(ValueError, 'ordered reference coverage'):
                    fixed.records.verify(value, value['destination'], placement['common_contract'], placement['references'])

    def test_original45182_projection_rejects_truthiness_and_truncation(self):
        import copy
        descriptor = fixed.selected_descriptor()
        context = fixed.reference_context(descriptor['source_rosters']['accepted_roles'])
        spec = context['historical_equivalence_parse']
        value = fixed.records.decode(fixed._read(spec['pin']), fixed.records.WRAPPER_CAP)
        fixed.verify_historical_projection(value, spec)
        for field, changed in (('matched_tests', True),
                               ('native_and_first_d_complete_equality', True),
                               ('full_gate_output', value['full_gate_output'][:-1])):
            bad = copy.deepcopy(value)
            bad[field] = changed
            with self.assertRaises(ValueError):
                fixed.verify_historical_projection(bad, spec)

    def test_executable_metadata_value_does_not_become_third_pa1_binary(self):
        from unittest.mock import patch
        value = dict(path='/frozen/value/only', bytes=222623192, sha256='a'*64)
        with patch.object(fixed.os, 'open', side_effect=AssertionError('metadata path followed')):
            fixed._executable_value(value)
        self.assertGreater(value['bytes'], fixed.core.BINARY_FILE)
        with self.assertRaises(ValueError):
            fixed._executable_value(dict(value, bytes=268435457))

    def test_current_pair_rejects_non_provenance_change(self):
        import copy
        sources = [dict(path=str(fixed.REPO/'experiments/evm_feasibility/snapshot_qualifier/src'/('x'+str(i)+'.rs')),
                        bytes=1, sha256='a'*64) for i in range(13)]
        compiled = [dict(row, path='x'+str(i)+'.rs') for i,row in enumerate(sources)]
        before = dict(source_profile=dict(compiled=compiled, executable=dict(path='/old', bytes=1, sha256='b'*64)),
                      logical=dict(value='complete'), physical=dict(value='complete'))
        executable = dict(path='/new', bytes=222623192, sha256='c'*64)
        after = copy.deepcopy(before)
        after['source_profile']['executable'] = executable
        fixed.verify_current_pair(copy.deepcopy(before), after, fixed.NATIVE_FIELDS, sources, executable)
        after['physical']['value'] = 'substituted'
        with self.assertRaisesRegex(ValueError, 'complete current report inequality'):
            fixed.verify_current_pair(copy.deepcopy(before), after, fixed.NATIVE_FIELDS, sources, executable)

    def test_saved_gate_requires_all_nine_exact_nonzero_matches(self):
        log = dict(path='/frozen/log', bytes=1, sha256='a'*64)
        gate = dict(command=list(fixed.SAVED_COMMAND), exit_code=0, log=log)
        output = b'\n'.join(('test vrf_shared_window::tests::'+name+' ... ok').encode()
                            for name in fixed.SAVED_TESTS)
        fixed.verify_saved_gate(gate, log, output)
        for bad in (b'', output.replace(b' ... ok', b' ... ignored', 1), output+b'\n'+output):
            with self.assertRaises(ValueError):
                fixed.verify_saved_gate(gate, log, bad)

    def test_s3_all175_rows_fail_closed_on_pending_discount_and_forward_edge(self):
        import copy
        value = fixed.records.decode(fixed._read(fixed.S3_MANIFEST), fixed.records.WRAPPER_CAP)
        self.assertEqual(len(fixed.verify_setup_manifest(value)), 61)
        for change in ('discount', 'edge', 'event'):
            bad = copy.deepcopy(value)
            if change == 'discount':
                row = next(r for r in bad['entries'] if r[7] == 'PENDING')
                row[5], row[6] = 1, 'a'*64
            elif change == 'edge':
                bad['entries'][0][9] = [174]
            else:
                bad['event_mapping']['consumed'] = [0,0,0]
            with self.assertRaises(ValueError):
                fixed.verify_setup_manifest(bad)

    def test_s3_setup_prefix_keeps_gate_control_occurrences(self):
        units = fixed.setup_units()
        self.assertEqual(units[0][0][1], fixed._pin_tuple(fixed.S3_MANIFEST))
        self.assertEqual(units[1][0][1], fixed._pin_tuple(fixed.S3_PUBLICATION))
        self.assertTrue(all(row[0][0] >= 1500 for row in units))
        self.assertEqual(sum(row[0][3] == 'json' for row in units), 5)
        self.assertTrue(all(row[0][0] not in (684,685) for row in units))

    def test_serialized_manifest_preserves_logical_digest_and_unit_order(self):
        compiled = dict(setup_unit_wire=(fixed.core.canonical(((1500, ('/setup',1,'a'*64), 'evidence','bytes'),)),),
                        unit_ids=('archive',),
                        units=(fixed.core.canonical(((0, ('/archive',2,'b'*64),'evidence','bytes'),)),))
        extra = (((682, ('/extra',3,'c'*64),'evidence','bytes'),),)
        ids, wires, count = fixed.invocation_wire_manifest(compiled, extra)
        old_ids, rows, old_count = fixed.invocation_manifest(compiled, extra)
        self.assertEqual((ids,count), (old_ids,old_count))
        self.assertEqual(fixed.core.frozen_manifest_digest(ids,wires), fixed.core.digest((ids,rows)))


class LosslessFramingStorageTests(unittest.TestCase):
    def manifests(self):
        first=fixed.core.canonical(((7,('/body',12,'a'*64),'source','bytes'),))
        other=fixed.core.canonical(((7,('/different',12,'b'*64),'source','bytes'),))
        for index,route in enumerate(fixed.framing_routes()):
            yield route,('first','repeated'),(first,other if index==15 else first)

    def test_all_routes_expand_to_every_original_occurrence_and_digest(self):
        storage=fixed.framing_wire_pool(self.manifests(),live_bytes=0)
        self.assertEqual(len(storage['wires']),2)
        self.assertFalse(storage['complete_preflight'])
        for i,(_,ids,wires) in enumerate(self.manifests()):
            actual=fixed.expand_framing_route(storage,i,live_bytes=0)
            self.assertEqual(actual,(ids,wires))
            self.assertEqual(fixed.core.frozen_manifest_digest(*actual),
                             fixed.core.frozen_manifest_digest(ids,wires))

    def test_reordered_route_missing_route_and_changed_index_fail(self):
        rows=list(self.manifests())
        for bad in (rows[:-1], [rows[1],rows[0],*rows[2:]]):
            with self.assertRaises(ValueError):
                fixed.framing_wire_pool(iter(bad),live_bytes=0)
        original=fixed.framing_wire_pool(self.manifests(),live_bytes=0)
        for change in ('index','wire','order','trailing'):
            storage=dict(original)
            route=json.loads(storage['routes'][0])
            if change=='index':route[1][0][1]=999
            elif change=='order':route[1][0][0]='changed-order'
            elif change=='trailing':
                storage['routes']=(storage['routes'][0]+b'{}',*storage['routes'][1:])
            else:
                storage['wires']=(storage['wires'][0]+b' ',*storage['wires'][1:])
            if change in ('index','order'):
                storage['routes']=(fixed.core.canonical(route),*storage['routes'][1:])
            with self.assertRaises(ValueError):
                fixed.expand_framing_route(storage,0,live_bytes=0)

    def test_live_caller_is_charged_and_storage_cannot_grant_fit(self):
        with self.assertRaisesRegex(ValueError,'allocation'):
            fixed.framing_wire_pool(self.manifests(),live_bytes=1900000)
        storage=fixed.framing_wire_pool(self.manifests(),live_bytes=0)
        with self.assertRaises(ValueError):
            fixed.expand_framing_route(storage,0,live_bytes=1900000)
        with self.assertRaisesRegex(fixed.IncompleteFit,'complete source-backed'):
            fixed.framing_partition({},())


class RecoveryCounterTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        raw = (fixed.ROOT/'offline-fit-counters/state.json').read_bytes()
        self.assertEqual(hashlib.sha256(raw).hexdigest(), fixed.RecoveryCounters.LEGACY_SHA)
        self.old_raw = raw
        (self.directory/'state.json').write_bytes(raw)
        (self.directory/'lock').touch()
        self.summary = fixed.RecoveryCounters.migrate(self.directory,
            consumed_events=(31,0,0), event_records=tuple('a'*64 for _ in range(23)),
            accepted_preflight='b'*64)
        self.old = json.loads(raw)
        self.context = dict(run=self.old['binding'], phase='development:0:0:archive',
                            route='archive', manifest='c'*64, source='d'*64)
        self.roster = tuple([fixed.core.digest(self.context)] +
                            [hashlib.sha256(str(i).encode()).hexdigest() for i in range(15)])

    def counter(self, context=None, roster=None):
        return fixed.RecoveryCounters(self.directory,self.old['binding'],self.old['history'],
            namespace='development',round_index=0,slot=0,
            context=context or self.context,context_roster=roster or self.roster)

    def test_migration_carries_every_charge_and_full_legacy_bytes(self):
        state=json.loads((self.directory/'state.json').read_bytes())
        self.assertEqual(state['legacy_raw'].encode(), self.old_raw)
        for key in ('binding','history','invocations','units','binary','seconds','slots','routes'):
            self.assertEqual(state[key],self.old[key])
        self.assertEqual(state['nonbinary'],self.old['nonbinary']+32768)
        self.assertEqual(state['failures'],[31,0,0])
        self.assertLessEqual(self.summary['attempted_bytes'],28679)
        self.assertLessEqual(len(fixed.core.canonical(state)),4096)
        before=(self.directory/'state.json').read_bytes()
        with self.assertRaises(ValueError):
            fixed.RecoveryCounters.migrate(self.directory,consumed_events=(31,0,0),
                event_records=tuple('a'*64 for _ in range(23)),accepted_preflight='b'*64)
        self.assertEqual((self.directory/'state.json').read_bytes(),before)
        with self.assertRaisesRegex(ValueError,'migrate'):
            fixed.RecoveryCounters.initialize(self.directory,'a'*64)

    def test_new_namespace_keeps_legacy_archive_and_burns_before_work(self):
        counter=self.counter();counter.validate_invocation(self.context,0);counter.begin_io_scope()
        state=counter.admit(invocation=1,units=1,nonbinary=100,binary=20,seconds=300,slot=0,route='archive')
        self.assertEqual(state['routes'],self.old['routes'])
        self.assertEqual(state['development'][0]['archive'],1)
        self.assertEqual(state['invocations'],2)
        self.assertEqual(state['nonbinary'],self.old['nonbinary']+100+2*32768)
        # An interrupted invocation is fully burned and cannot resume as success.
        other=self.counter();other.begin_io_scope()
        before=(self.directory/'state.json').read_bytes()
        with self.assertRaisesRegex(ValueError,'pending'):
            other.admit(invocation=1,units=1,seconds=300,slot=0,route='archive')
        self.assertEqual((self.directory/'state.json').read_bytes(),before)
        counter.finish(state['generation'],failed=True)
        state=json.loads((self.directory/'state.json').read_bytes())
        self.assertEqual(state['failures'],[32,0,0])
        self.assertTrue(state['failed_development'][0])
        self.assertEqual(state['nonbinary'],self.old['nonbinary']+100+2*32768)
        again=self.counter();again.begin_io_scope()
        with self.assertRaisesRegex(ValueError,'terminal'):
            again.admit(invocation=1,units=1,seconds=300,slot=0,route='archive')
        self.assertLessEqual(counter.io_summary()['physical_operations'],6)
        self.assertLessEqual(counter.io_summary()['attempted_bytes'],24582)

    def test_exact_stream_slot_tracing_and_second_round_reject_before_debit(self):
        before=(self.directory/'state.json').read_bytes()
        with self.assertRaisesRegex(ValueError,'tracing'):
            fixed.RecoveryCounters(self.directory,self.old['binding'],self.old['history'],
                namespace='production',round_index=0,slot=0,context=self.context,
                context_roster=self.roster,trace_io=False)
        # An exact same context/roster cannot be moved to another slot.
        counter=fixed.RecoveryCounters(self.directory,self.old['binding'],self.old['history'],
            namespace='production',round_index=0,slot=1,context=self.context,
            context_roster=self.roster)
        with self.assertRaisesRegex(ValueError,'context binding'):
            counter.validate_invocation(self.context,1)
        changed=dict(self.context,phase='development:1:0:archive')
        roster=tuple([fixed.core.digest(changed)]+list(self.roster[1:]))
        second=fixed.RecoveryCounters(self.directory,self.old['binding'],self.old['history'],
            namespace='development',round_index=1,slot=0,context=changed,context_roster=roster)
        second.begin_io_scope()
        with self.assertRaisesRegex(ValueError,'terminal failure and named correction'):
            second.admit(invocation=1,units=1,seconds=300,slot=0,route='archive')
        self.assertEqual((self.directory/'state.json').read_bytes(),before)
        # Even semantically identical whitespace is not the exact old stream.
        (self.directory/'state.json').write_bytes(self.old_raw+b' ')
        with self.assertRaisesRegex(ValueError,'consumed legacy stream'):
            fixed.RecoveryCounters.migrate(self.directory,consumed_events=(31,0,0),
                event_records=tuple('a'*64 for _ in range(23)),accepted_preflight='b'*64)
        self.assertEqual((self.directory/'state.json').read_bytes(),self.old_raw+b' ')

    def test_round_source_binding_cannot_change_after_first_success(self):
        counter=self.counter();counter.begin_io_scope()
        state=counter.admit(invocation=1,units=1,nonbinary=100,seconds=300,slot=0,route='archive')
        counter.finish(state['generation'])
        changed=dict(self.context,route='pre-gate',source='e'*64,phase='development:0:0:pre-gate')
        roster=tuple([fixed.core.digest(changed)]+list(self.roster[1:]))
        again=self.counter(changed,roster);again.begin_io_scope()
        before=(self.directory/'state.json').read_bytes()
        with self.assertRaisesRegex(ValueError,'round/source'):
            again.admit(invocation=1,units=1,seconds=300,slot=0,route='pre-gate')
        self.assertEqual((self.directory/'state.json').read_bytes(),before)
        with self.assertRaisesRegex(ValueError,'context binding'):
            counter.validate_invocation(dict(self.context,manifest='f'*64),0)


class CompleteFitExecutorTests(unittest.TestCase):
    def test_indexed_pool_preserves_full_escaped_pins_and_occurrences(self):
        ids=('first','second')
        pin=['/home/fry/artifacts/quote-"-unicode-\u00e9',17,'a'*64]
        first=fixed.core.canonical([[4,pin,'evidence','bytes'],[5,pin,'source','bytes']])
        second=fixed.core.canonical([[4,pin,'evidence','bytes']])
        pool=fixed.pack_complete_fit_units(ids,(first,second),live_bytes=0)
        self.assertEqual(len(pool['pins']),1)
        self.assertEqual(fixed.expand_complete_fit_units(pool,live_bytes=0),(ids,(first,second)))
        changed=dict(pool,pins=[list(pin)])
        changed['pins'][0][2]='b'*64
        self.assertNotEqual(fixed.expand_complete_fit_units(changed,live_bytes=0),(ids,(first,second)))

    def test_binding_and_actual_unit_owner_are_charged_before_construction(self):
        unit=SimpleNamespace(invocation=SimpleNamespace(metadata_charge=lambda:fixed.core.SEMANTIC),
            declaration_charge=0,declarations=())
        with self.assertRaisesRegex(ValueError,'full caller/binding pre-allocation'):
            fixed._fit_construct(dict(role=38,constructor='current_source_role'),
                {'all_sources':[]},{'declarations':[0]},unit,(),{}, {},0,
                consumer=False,input_start=0,read_charge=None)

    def test_compact_repeated_long_pin_is_bounded_before_expanded_row_growth(self):
        pin=['/home/fry/artifacts/'+'/'.join(['a'*180]*5),1,'a'*64]
        inner=fixed.core.canonical(['long-unit',[[i,0,1,0] for i in range(500)]]).decode('ascii')
        pool=dict(schema='fixed40-lossless-complete-unit-pool-v1',pins=[pin],
            kinds=['source','evidence','binary'],modes=['bytes'],units=[inner])
        with self.assertRaisesRegex(ValueError,'expanded row pregrowth'):
            fixed.expand_complete_fit_units(pool,live_bytes=1000000)

    def test_closed_dispatch_covers_exact81_roles(self):
        import pa1_fixed40_constructors as constructors
        names=[fixed._fit_constructor_name(role) for role in range(81)]
        self.assertEqual(len(names),81)
        self.assertTrue(all(callable(getattr(constructors,name)) for name in names))
        with self.assertRaises(KeyError):fixed._fit_constructor_name(81)

    def test_historical_scope_bound_precedes_read(self):
        pin=dict(path='/home/fry/artifacts/NONHISTORICAL-oversize-scope',bytes=32769,sha256='a'*64)
        with mock.patch.object(fixed,'_read',side_effect=AssertionError('must not read')) as read:
            with self.assertRaisesRegex(ValueError,'pre-read allocation'):
                fixed.historical_gate_scope(pin,{},live_bytes=0)
            read.assert_not_called()

    def test_complete_json_width_matches_encoder_at_escape_boundaries(self):
        for text in ('\x00','\x1f',' ','~','\x7f','\x80','é','\uffff','\U00010000','"\\\n'):
            value=['complete',{'value':text}]
            # Width excludes the canonical stream's terminating newline.
            self.assertEqual(fixed._framing_json_width(value)[0]+1,len(fixed.core.canonical(value)))

    def test_framing_manifest_failure_precedes_open(self):
        pin=dict(path='/home/fry/artifacts/NONHISTORICAL-representation',bytes=1,sha256='a'*64)
        compiled=dict(units=(),setup_unit_wire=())
        with mock.patch.object(fixed,'invocation_wire_manifest',side_effect=ValueError('manifest failure')):
            with mock.patch.object(fixed.os,'open',side_effect=AssertionError('must not open')) as opened:
                with self.assertRaisesRegex(ValueError,'manifest failure'):
                    fixed.framing_partition(compiled,(),representation=pin,read_charge=lambda packet:None)
                opened.assert_not_called()

    def test_partial_program_and_unknown_constructor_are_rejected(self):
        with self.assertRaisesRegex(ValueError,'exact81'):
            fixed._fit_check_program([],[],(),(),())


class Round151QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=83.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 84.0, 'baseline_used_percent': 16.0, 'consume_percent': 10.0, 'floor_remaining_percent': 74.0, 'observed_utc': '2026-10-24T01:12:42.440000+00:00', 'resets_at': 1793382070, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '21c08e37f50d35f95494477558cc6eb94d693db13b6934fd8a395971067dc5ae')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=10.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=74.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(74.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((74.0, 0), (73.0, 0), (83.0, -11), (83.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_twenty_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_twenty_points':
                value['consume_percent'] = 20.0
            if case == 'rollback':
                value.update(remaining_percent=85.0, used_percent=15.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'84.0', b'83.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)

if __name__ == '__main__':
    unittest.main()

# Actual source139 offline inputs; regression data, never runtime authority.
import base64
import pickle
import zlib
import pa1_fixed40_runtime as runtime
from unittest.mock import patch

_ACTUAL_WIRE_FIXTURE = {
    'permits': (32737, 'b59c592d067e1766c7f58a2c8acd853ac02668b127caa602b66278e9a747cee4', (
        b'eNq9XWuMJFd1nume2V2ztoPtIGGjAMZJMA613VX9NnH8WG/Mznq9y2Ztg71Oc7vqdvf11Gvr0TtjQWwHDJEwAYu1'
        b'CBIRwnkQjBOUH3lIjpAcGSkEghGRTJQQjISUBAgyIYnQOomc+6jH7Zqq7p26t4Y/eHa831f33HPO/c65Dz+y9uSL'
        b'D6/Q/73//PWPH29MHQs2xt52A3gBGgM98Bszb6zojh14+CelqQK13WsCRWtqXbWp9hrOyEQTECDHVizgusieHJp1'
        b'Dj3oO/b5B85fv9G+8/KVYyvw8WNF0C5QFWS5JrSgHTCcnSShjQLFNYGdAv/9mRoFfvciYL+VAzZGW9BoNxvOeGwi'
        b'GypjjK07oR1Az2/4+BtgwnL8OzXCcceuOXwYhC42iI3G0A+UmZpCvu8FArlREtINsb11ZicedXrZQmN4eHxGhKqO'
        b'EtRuA/h4Yhs23AramuKHug593/HSCZ/1UpM/+7VVavJTu2Zh+DGN7ljYMoYy9iB8KDX2xuERxl+Fj9+7a3zfCT0d'
        b'4slzPDCBygyYyGA20kPPw46VGuqrN5IhHC6gaDcVYFjI9xM/HHW7MU+/AWcYS2mlNjn+2zVxuEEKt7YqDNfmvOJR'
        b'CXBtDk58sO00Nxx/k4Sv46biMxK+rp/C/VACHDezT4oPttNM4d4o/nUdzlFaEuC0FO6f6uJwrRTuPglwnBsbEuA4'
        b'N55IsF03hXMlwHFR4UuA49z42JowXJdz4++KR0WXc+N3Svg6zo3PSfg6zo1/RwIc58bPS4Dj3PgLEuBSNz72E3E0'
        b'zot/V8LHcbn9LyXAcUHxWXG4HhcUL0mA44JiY784HBcU/yrh67ig+AsJcFxQPCYBjguKQ+IJpcfl9j+V8HVcVHxU'
        b'AhwXFR+kcPeWg7Ox3p5BJYA21vXOQ/j/AC4m3ADYOldPrR9cVE8toXBc6DFV3+K++pV1KZBtLgA/SSF/TRhSUzzo'
        b'Q29Gf0zhR/vlwLfy4b+5Jge+mw9/HTXO3YLwFF13ZtDbVnQTICsl+Kt9i0rZJQQxKMEPceXNKsNsqXn8v47I4Qgc'
        b'xyzi+BKdh+PlOHwwgwaNJmS7Ia7DuaT00roIMMS1d4B0huvPAb985aLWBAbGlbtrwoD1TCLIQWOKSOG9raltZabG'
        b'P6WoF+qLGkAXieqSur7Z5IJ0dLM8WG61/LZEWG7VfNP1S4IyH5a51qCvMHykA1OZAPxv+ToOpRT+j+8g8Lcvhc/z'
        b'iV60FKiccc+uS8DjrHrZPgl4nDkvLFuXLwKPE3A3v2bJ7Bj4r5iOS3qUaSgBEwFfIZ0sJwxIkHoApwEPzhA8N9ec'
        b'u/z1ZeD5P8RrKelQInuizFrYRWxgpvD/cgWBf5cA/Ig01gBOxTvBX6WmuVMAPO0qzjjdd+nfEdz7BHDxt8It0qMc'
        b'beOA0B17jDwrs1h9f18Zy8fx0MSGcQLSIXXz18KvUvgTJeFVDp4sKF7IQR+kWfN0SWiNdsujtSP303+B9gXeUxK/'
        b'xfC5KMsl+RYluackSTuOpSi8AF7TJrY11879QE3ETJ2YwYOu4wWJJEnxPy+E3+Wcn2bsHSJhe1XEQXtzv06W/RT+'
        b'b/eJfH4/ljaRlXBqMHAK8lP8z9VFAmCgjDxkTEjwetj0877zTG2JzlwErTaxdsLy70H8/Tid5RLcQteQMyUJ1Ng2'
        b'JPE4OPHY1E45QfCMSKSpmmKhSSTJsWzbhEHuYJ6rlUmnMUlr7tcuFbXhyEAzNO9O3foS2byQpa24HnK8fiuNuhxz'
        b'vSwSEWon4UA2ChCRSyHwjBTeFILvJvDnkAcVE41x9rO4eP5lIQv1iDfpHpGBnmNS38LLQ6hj6ccF3UepOPv1khz9'
        b'ObVC1AvTMXgyHDOcn4sP0RCZlmQaRPMMYwpClkiN+U1friW8LhAuWjMKSxyfi5yM5a0HSpKo+Ne+7iEXz0viA/lE'
        b'YV1kNFo8Gm3haJy6gE9rLUXHE0M2TOccLoX/6apAbtHaHPyiQdwjku81EvfQBVGihGdDNAMmnOs9fYcO470lCbo8'
        b'KG+n3HT8M3Qwo5JcvTl8A5oBWGg6XyT5a+kq7/gB9BQX2blj+oqQGwz4EpYmNyaI2Li47o+I4Go1lbOhQ6yVyZ4p'
        b'/pdF8n9LzUjSHbL9jEj+b2lRRUOOX8CtoCD//2S1TL0Xc7QS9JFjbO+w/5+IeG6rHc3yNi0NHJv8qYHAxMauhXRl'
        b'DJAZelxMviSijVpYvpOimOXe0CyyF9vtGZYk6WZJyCoWjzJrvaeExtOLIvGc420uUGF3iCxerX48DnIOKlqTccmP'
        b'BxOf30qIvk4Nd39JokGMS5U4jhOcy+CWi9NKJn2xHaWSQhz/2nR0KvEtCwXK1HE2C/zgW6sC1WhbzeHJTv8TNQFP'
        b'a2sMNNh2mS7DK5qvAz4Zc4P5s1UBT2u3EnA6OZajbyoI67/MzPyb0My0+YNYfmhZgOaFnKT8jWU7Twt5Onk82an5'
        b'DTo1oCRFN48CV8NwK3epfEpEarR7GJnZiJ5gw4kUeHhy7Hzb/aNIkLb7RVxZ+7EqdlySZjBHgyWUQxoJqXbOM+I3'
        b'VwWW0k5TAUEALRc7N/A2oZdvvB+JqIGOmuXIGu1hkeWgo3FmYYt17hj+U8QBOi2ehJed2bF8sbbo7OcymnYkzMam'
        b'QySz6fgQS6gU/cNClupc3ML5zrqAcOp0FTO0QVrA6lOob6bgN4rUL50elaw4HLHaTynwEs0pM5GM3+nHBDptZk9w'
        b'+qc7hLRFnJL8eFWgt9AZxCQkKLbT88954f20iCrrNjkJRpay5PD4BNrRFnNKdVtNIEK6ahFVNkL+QKQM62oK2TVM'
        b'pWVRf+QJkUjptjIscw3elOTqmsAC1uWqAGCaNC8m/hb7drbhV7Li6HYyA8IzhMtKmwqoMfKw87nA53TT91cF1rFu'
        b'NzUcDn8LKMAaoUmIsLPTTpph4eI5szcbCHkfrgmgh6juTHNzmu1SGpsmN1iSpp9uQO9cBPK7DnfUBFJFd6CAMJg6'
        b'HjHdfLfDQ1mmD4lot16TYyKThMcEPT/qPxg7XP9xkenqqQn8kkaBIqKneho3Jow8wgazsKOTqw6ZLJVN7SXLxl5r'
        b'x8DYiHLW8k/XRaarrYw9YBF1eFFren1dIOn2OplKPp6ukQeBPk1ZrrlCYLHqdVliQGSrl56Sw2twEHqQ/C0EMoXd'
        b'pSI9914vsR5x9fimDSfxckP51TUBqd3rK9Hhv+RsUd56/+yaQH7qDSJvwJUYzeyOHvpJ8OYP6nX0mN2kHGG/mfEI'
        b'l+OIbRvtfXPHZq4W8Py+qthYLOFRsVWT9WY2EU1ZOeN76YBA0dzXkng2wTYeY+6cXUODSy9J0Uo6Z9E2LxfQuSN6'
        b'47qAhO6T4wI4wpBOTEhOUUUew53z+ZSIRu93KOrCmvVZkZWq300+GQ/lwShgiRPwWwAp2ZU0ok6WJOvR0eAlZEL6'
        b'/cD1Q5MTLYaQpfoUe4xM6G9jj7YUZOBf4MUqJXh7XaC11B9Qgik0DX4rLm9GNkQ24AZNyoMsKwzAyITJpnLmmugz'
        b'IoMZqJTEdXxEsyhpz9Juc6yZU57fF5mVgcZmJTRNDA5of1GfAm/CDeTFukDOHLTYQGLY2KuoC9M+o0vgMuvdcyJq'
        b'YdBmlDEuITdBaOvT/K7pl9cEEtogbjPgYpoewqTLABuqm5vQrlgX0JKDLh2bAUfkgnKksnKl/zfqAupkwBJBQoCL'
        b'J9qbI9Po4H+cH9HH6gLF2qA/T4WcaBOQ1k7JEceU7WdFtNCAJQl/2w6mkKiSSPAje+ZEN6jzEsZtAv6oNZuZEUb5'
        b'Ilk0MqdRXqiXb6ppTXVJU+3FtfLuh3/EXx/A+OwRvXudv2y/sK989tOaSdIYoy1aCeLkl1/hflKgv6I12xFmt60E'
        b'JGSJSkib+inL++vlU4TW7MQjyZEJcfU5t4i/4xKROerGdOw4Juu25Hn1VXSOrJI0vbQ7kOmBzblInnP8eL9IOPXT'
        b'7kfEzxNOsUGd8Zhr57+WkDklyQbYmI5PNijJMswTwS1cu9GkSOrQ3HEeoeM0ylGT00qOCXFZ73ikAYKZ5soN8kUZ'
        b'vlNU/Zsl+bT0aFbBDHKkyow7ef/cwfJ9C01tRUGINQ+2tRHqtDm4eKRfOSDgQWp7/mbFMrJXSh279mAAkE0qtynC'
        b'kjRqqtJDb9Cfuwpw8JIljVV+OmJ8lnzxT2xUmtqMN5zgeBy5a9SP5KfqusuWXLrAUxBXGUWXJDQO782rEvBa2Xuu'
        b'gnjc3c/PyMDjrkOt1yTgcbc/PysDj7v+eUAGXj/7poQgHldt/7yE+Whxl5LW6hLw0lg89m0JcFx47JcxXC48XpWB'
        b'x4XHX8vA48LjggR3aXHhsa++pJ2xEG/u+OeM9Bv4s5+IFmK/WoxNzlzbc0k3+sL+IH+9X6eIN5ZAHDSzLwfcXgZF'
        b'zbvvsbK/7DgHBZeVj9ZLIxbcT/7A2pI7I8WI7fgEYSSls7d1nqgvuRlbDN1Z2Et+pLwVCq5RH1hb0sopRoyPCc4d'
        b'ed5xtPL+tSVKppggORGcXiTeWVz+zb4lFwOL8QdpgbxzDreo3LttAS6tRLPvGBSE6b/TgOjuEq2fhuix/ygFwC0z'
        b'Xysznn5BOP75eqnP4ZaVG2qlEDjd8Z5yCLzSoOF0dHcIvtJvK332boCtIxNlTPODZbsH0V2K1kDxQzc6XBFxaNpp'
        b'VVO7au++6Mq1gixyh1vlH2d7/rEll47Ty36Fd5hb2cOVYnDc8v62mjgct7r/oYSv4xb3ByR8HeeC/72+pNCn17Og'
        b'TWs1GE9yi7z9SDde8b8ZApPziJnaSOrENOvRNjl5CJF7pXBlhT08OZFLzt4pVFX2pET2mujKLeRhRFQF5aBZ9IDF'
        b'yiOE1JNKOndfNTnCkCq7uSFjKz9UGTvdwlFOnjqxceTw6aMn7lLuOnFaufXkyTuPHrmd+4rXyp/rJW85YLMvbkCW'
        b'oIw3YqNuId+RYHa2KqJzyCYgtvKRk7eeupWYOTvOtzdIk9d3cYz5DfJkC9gCWMBbjUMTFJDz8D6R9tGpe7qePskO'
        b'cRb8NXLDwENkhn2cN6zhGAIfjfBqEWw3fBu4/tQJhmfxF6Mxgl7jMPAmziHT0TdpcKun2BOncuEDxzLpkD+xxjrY'
        b'csB9T28A34X20HVMpG8f8nx26vhmtkdTDcuQzbQfs/n00RhbItsI2Q0TzeAQ+98Qosk0mA5nTgC94TlkG865mJnd'
        b'ebaqYh6jcQExO+hZ2ZDH2MAFzH9UY13fipjJA725vE9Va2ob2QUD/lyN7QBWRIw1NjLwr4ftwbgfc/7zHnH2xn1t'
        b'rzlBH+w552iggb3mhJ3maM85B+3uXnOOR6PWPCeqnpMcq45Jv1tjSrwiUp9q//zc8Hy1ScnHAmdR/q+MOICLs2F1'
        b'xFPkGQtGPKmKOI/yEzXZKmaO8hwKbFxvxmyfrsSN2YnsoQ70KcRhA03Db6Rd24j61SvZjmol1PR6zzDeKI8lG63t'
        b'XcmMHnRNsL1MtP3vahWG5riLZduFqoe9QLj9z2oVkpHjLpRuP63a4MXi7RVK/WB11Lny7TW1vWLlBdzesfISbu9Y'
        b'eRG3d6y8jNtDVk7I7R0rL+UY6+ZesPJi7lJKe7Y62kVy7nW1ihNVsaC7UHWOLJZ0r1ROXSjq2KhRddR5pOzGvVEd'
        b'aUbYvRzdepPHZ0BMBY3hFAIDegnRLZbs3h4hGMbPKUcs99zJLl3KYyEHzrwZzgQZph/9ksxGKGEyUZLL2T2ZMxLB'
        b'2RW07Oxfca9snkhXZ4xVu192sbJIvyOvIjYPngM4WbBKJWY7+kX2RoZMtmCawH/9KtlxE42CZJ+E5SOXyE47i2uN'
        b'LynstqNkvuICY/0Gtmskl7Cwqvi9n2NXTuTS+VPsh8Y81caJC2zDVfbkFcuB96lV8BUvxDf9ouykXrT6/t/r2W04'
        b'uUyZpPvwgF1pkENC9sP9Bn02ArsGpjvkblO/uPuy9RWZTsiIXKAOuW3iiOz41R+WuZCkVMlRxYjnh9+TbzvCE/1H'
        b'L2Pbnf9UfUVmg3oH0XDuCbaI1QgYq14Za7TxHBN+r822c98rlZBcZRzmOORV18pNVBxbgVe+4Si7GlEJX9Y1H/1I'
        b'Rabc6Z/H3/CEfO/McmWd5dHfYt751iJW8s/ZAwPdR1Yv+i8lxwBuWF0YBfSv6B4IuD8n4QRtP/Rp0qX/jYahPgW4'
        b'9ImEr8/uCjdI1meqMcnH11+3UMkvp3NHY6YChtyFuWh17qwwA5zYDXz0H2ql4PGL10N6IAf/aEA92e94MzvPKwfa'
        b'cowY+K6D7O0EOcDudNsnx3mGNv74mOLK39y11RdQZJbY1XezBzukgg/Ze50xR0+thAOnTTMx0mMN9iSnXA6yyRP7'
        b'p1dj/nmmAo4hSSnxnGxc+wPGdLdkJs90Y3Mdo257qwDBGE1isD6VaTeVBTOSYt58mh2OLwnEJbMY8cA/EMRfKYvI'
        b'NRo+eDk7d1oSyYIBwPku6XwfNQjcXbuBI8+5+pH5cWpr4OnMG/LzhwnyzQuRZxBrea/h4cXHN0as+RXJEHJSNdnx'
        b'oWcWb9ktlO6YoWUPx8BCZpIEjlxDsDq7xUqcY+O0xaLipt1DxBoraea8dblX5AOhgLwJ6Hgx0sdPl/0kx2UvSkSj'
        b'O/80k3y3l4BykW2TR0WGvon0JCMa9PrDO3aLNx6jYRggM4a59i0YJjz0/65ROzY='
    )),
    'caller': (38436, 'a2af80571d2183008f8ced48698e364a68e80fa7c3da9aedb5389d9a381fcece', (
        b'eNq9vVuM58l13zdLckmJ1IXUhbrL0iq2GInDqeuvquRIXJmibe1qSYqmLtbFk7ru9Gp2ZtzTs+JGEUw4lmjTbcCI'
        b'WkYSK44RIXBgIAiQ+C1AgAB6SB6SlyBGgsSOjeQhCGAIfjKcTeB8vtWzs70iVxDJmdXltzM93f8+v6pzvuf7rTp1'
        b'6nPv+u3v+FvX9v/8+sWHzt99v9+aL9eL8x+q/exBvX39/t0Hp31aX67/6snpvN7r7dvz9PrpfPF03r9/cvfO9Vfs'
        b'xfnTp3fvnt2/0Ae89948ffnk7Oa9kzv77++6V89uXZy/cOPW3ZfnjXX66o16enay+PT7N145XcFcfu6888q8fffe'
        b'vO6MO6xx/sY6+ezZA/7ldNZx/fJD7/PLPvLS/bt3+I3t1bN5/+KFZz6Hybeqi8fF+bPDj1x8bq24bHIqZbphW+qu'
        b'HMl0l0PIzrvDjJDXiN1Gb1I24ziqW60d4eLB+Qea3uTstN67+RJvfqfevjj/Br3C7ZP2kZu373Z94Ws/dff+yWc/'
        b'pRf7rYvzn/qKX+2Lhlhj+V2PLLje5rrLt/Gzd/Zr3774zYtPX5y/597pvFdPxsUvX3zoC09fu/bB37r23Bf+j3/1'
        b'a194+pP/4I//0M994elP/3fPv7c/94//s2+79tz3/eV3Xnvue//mJ56a5++7dXL/7O7pq69Pza33nD/31qb3uy/f'
        b'uz3P5vV1cvbQcltuPPwIZwO2vv63yym59bUv/Mt33nrv+bMhl9hGbceR65zraM4nm4/UjjhSXDHk7r2proca8lGD'
        b'Ly2GUmovxY7DWmbhffdO766T2/OKqS8/zlG+cfrgztnJy/P6w99z/VOf/vinfvzTP/6Zn/zkJx69ze9+Wm9TPO7C'
        b'R45aTKzhaD6GYVIYdjheaNnoVp1muLSqmYdvlcHxzfZqXcjG8zbfePmrb56MyW89exVPtWasOJpp007TR8lzNnOU'
        b'bNKKJpjDxmiKPUoY8Ui9DtNyOlyILc9lfCsX5+/85M985uLWN956/29sl3jdWfUVnPhLO857b88Xa7/qAD//FqN6'
        b'/V611+/768ZWG5Kpr3tA0qhOBuTG3bVun9y5dI9+l+Gcp/dv3D+rZ/PRAP7jd2gA/czT97LmLHEt71puYa7RpjPB'
        b'hVXW0efw6eh+DDPtMVe1h6tEbGx2hcQAvvvkzr0HD/HlA3fq2ckr86b8s56e6Ffpyw/x4Llf/RtPXXsdcz751i5T'
        b'x8snl/C13xBfff0N843LX3D9jV9w/ZX4EHXegJqQk5092CPmWOKxXM1tjmzHGHG6FCYzVWxqnalLMVtfayqrKwCc'
        b'NWXwVt/w8E0I5runl29368YL/+iHbpnzX/rKLL9fX5kDax+9woPTU1wOCL13u756/RX3cGoc9tvGzx+lp25MHT1W'
        b'a10bifeaKzu+FjG/xBRi8KHW2HvpQGfE2VcszMqluT/1Ocz95a/S3HVyev/s+vjD7DXJhjqWnyn1XotbKaUajzJD'
        b'6WYA5eD9ZB7yMWPtxD9fjNm01ULTbFw8+E1we/9GUPzFh4P9359j/fNfmfWns999ZZ6+Go7rl8P9EX2ubC1r5MTU'
        b'R+/j4VwpGScnjOdaE5vjskf2vfc4kwk9j8H/u7q8tz1MQATf+LpLS8mxD26fPTT2f38aY1/4aob6bN65vkPpkUfL'
        b'3GZbCa376GeI1aXobO3dV+OXmaPm1fmsNez0rTOctdThivEdsKrBmC5XJi5Ot72XrvyhS4t/4P/C4s98lc5x+dGP'
        b'kNq8YTipO5NabJl9HCWtMpo9avO5A9Y5YjxvUfuxmnUeJPX4CKjLyx3uaKa+7sPf8wTMtG+YWY5inS3Ggd1tOYxt'
        b'o3jXS3OlrgUXwUMq1oMjZIxeGGrXre+4dWVSXjfzw0/AzCsR5ir/24JlLMOKieELdtqQxzKlHy0dJKBlWhngOOAG'
        b'hns7iC4SYjm6e2SmeQJm+jfMrJMs0UMn4XbTGsgUls1AFDSiuDTDMUZLNvkjhcLsR+OjHRZ+EQ7jvXuSoxneMJNB'
        b'8z3N2nozneA/6kFYZQcqORlWMHHkkSzU0/TgmGg+ujfIxOKHHuHr9Sdg5pXYB+GB1TgJnJWY70hEtZIOjWxpwxjC'
        b'Ki3n4SlEfa0e0E2xr+7iNMWU18386Sdg5vGGmcMDoYxLgDB5m+MBNI2+5J+9whFMZ4I1hoN/buvIuK1rK88W4sEg'
        b'v27mp5+AmemKmSvlzmTGGTxDaE0kWggn5MjKfvEO+Yi1MnCmgViOjAuk+kEKdrhLe93MTzwBM/MV3wxQzIYmgnAt'
        b'a70LLufQPDysmsygRpuH8XlEB/MljS7o7zTkWBx6mvgkzSxX8xJc0YKHUOM0ezoGVKtVGLitiWAKqxe8oMK1uyWT'
        b'OpO9MfbyvzM+8s1/8H9i5s88Zni/koZagt6m7pXN84w+1wbYNxFDE7sjHWV/+JjwAQt7QUYdHopg4pwk3Deo1BOx'
        b'80oeQmTVZVL08I6WGNHSW2Acj9YQIzGhPxAxWXQ1+zgIfTAf6B/JpwKtfaJ2Xk1EzkVjVodCkaYtYLP/RooJCInj'
        b'iKT92Ix3Zk5AP6wQRpgQ1pgA/GO9bud3/tMnYOeVTJQ1tZ7c3SssbvUIry8Y1Zj6TAY3ixFkuOFP+G1sZflsAtwK'
        b'xHUzjdft/J/+yROw80oqqgQzUDMTjpncWAOthSI5+LTpY0RdhhTanNGBTyvyOjNHkuXMxYn7P1E7r+SiCFwDQs3G'
        b'oy3YMfHvhvV2iaxBRkLvKTsIXAYEvHGQvMWfYppSjccjkH/3P3wCdl5JRtCIlPjNxsPTUEmkR7R66ySlYIim2kYF'
        b'UeFI2dRAJDloc8RZFsGP1r94cHZx/vUP1wTW6Zz/1nyoxk8e6xrHQzVVzMMvX7/8XY/k+f/yMcnzSuD4QspadoH4'
        b'4K09fBu2FgYcxoI/2DAa+aHj0+OYduIkoYZRE9zl0PoGyvvs5vzs7A/Oart9+TY3nrv16vNfz0z8lRu/evf0V+7f'
        b'q33ev3H64P5Z/Wy9Pl95+cb87L15evIyJt6/wd9vIo7un7ST2ydnr964f6feu3/r7tnNv8hrnayTeXrjrJ6+OM9u'
        b'jNkevMjz3qMPu/nF33w99olLVHzaLphhecgiUiAeD5gYma8d2TWbeskkQNgkFL0O+NcaSLW4xmQOi/U5IoDTHCh9'
        b'LUyd3b17+82T9uKTmDRrr+s3/cEp+3e/Zi9JBUuiO3o4Ym7zQLzB1RoE14KSycOVjK/IeHQ9sX+AObhetQNexHh0'
        b'AgWBeXLn/tnpg35GHNzX+snWhV/kl795ces33/zSyOh3vVjP5sUn+NO3PVzCeHHemaf17O7pzcsf1z/yg19irUY/'
        b'fn/W2/vHv/VyAfGk19s3ibSX5jZn/9O7H37Q89f4pN9+/qn9fMd+vnM/37WfT+/nu/fzPfv5Nfv5tfv53v18335+'
        b'3X5+/X5+w35+436+fz8/sJ/ftJ/fvJ/fsp/fup8f3M9v289v38/v2M/v3M/v2s/v3s/v2c/v3c8/tp/ft5/fv5/P'
        b'7OcP7Oe/tp9/fD//xH7+4H5+aD//9f38of384f388H5e38+P7OeN/TT7affT7affz7CfcT+P/Uz7mfez7OeP7Oef'
        b'3M9/Yz9/dD9/bD8/up/P7ueP7+ef2s+P7edP7OfH9/NP7+ef2c8/u58/uZ/P7efz+/lT+/nCfn5iPz+5n5/az5/e'
        b'z0/v55/bz8/s58/s58/u58/t58/v55/fz1/Yz1/cz1/az1/ez7+wnzf389/cz7qfbT/7fg6c7H0Ply3u1ZPTS097'
        b'bXvaa9vTXtue9tr2tNe2p722Pe217WmvbU97bXvaa9vTXtue9tr2tNe2p722Pe217WmvydPO33O/AwX+2D7+vofh'
        b'0e6OV/WFl97/1LVr5+/WwtLpuJBHvfQBviInufwDs3z+wdN5Vk/uYHW/dXKb5907Z6d3dzy99EH9/LddCSoF6pu+'
        b'49v5jjce5x/8g9/7MOrO22MC7fun/Qahz7ff3ABz81a9M+6u9ZHT+xcvnJwiQHOI0yGH3SQ5xgjdJwF1Z2B2g1xT'
        b'Q/SdXANox94bMBZI/OuoFbYCW734PJDy0nfoZdZjtLmd3Hlru+8/BQY3NFMOJTqoymjeRZJjr94IlFOcB/qYrNI9'
        b'UF0y+X/AqxwMtSdXkLNX7P7px2i3dqlk4n/1DniKlcywoWBSCXNmV8ORYqkiTRn+P5BW02UUaW9N1LrNGguvwxiX'
        b'Ya6Y+EuP0cSHTv+rJ2d35v3729oP/Nz5s92FnuJAacSeikfsH9BqH8p0BuqR6txaOaNaEQWxLljiUY215HFS37hi'
        b'7S8/RmvHvF1fJdhuzTrm6aW5z74MWe1prWJnNI6MO0KHkqLxRj/wZQigxeg0oIABPRgWmnRCG31dHZZ7+Kvm/oXH'
        b'aO6J9mFeIZpf35qTvf/sh8+fDWHaoK2sapw5cFeCy9XUUSSrHEErPseAn7Tsly0RyhDTYXHZ5aEYM1yx9xceo70a'
        b'1jfZ+rM/hSt0M2E0LiXrIGOTcILrMNuQ0YE0gAIeOMDqhFj2vkxfx4FMbAd6JZX2hBz3IR5ctfYdv8jIHikfI/i1'
        b'rBkm8Ucn1b+KsY3gm+GAZmIkVJJoixBKR7Sl6kUs7ZusfQKoezp/tZ6Om3z59rj03p/8zyH9WNg0igGPXM21PMZy'
        b'E5Y4BxgcGyRyhZRMNDYBEgnyGI4OeuQlHfOEvOGhsa/cRVBsW//614K0WbvDgdjvI5QjJUbTaSuqgBYAQHN1DK27'
        b'JMQXzmzmgOT7lFOpMaV+xdY/91jH9+zWpZH/wzczoG60NAOhj7pHc1jrQ6je46fJIWqHG30CBTn1OVoZKx/OuSzh'
        b'VSJZ7YqRd59MGuu135oP/eDGIwm8X+BffRPayIVUoxujLC2eNu37zdjtCjaWozhScVzdaWRdN9p0OVZdMyYcfSB/'
        b'n5BH1Pv35p2b9+7ePumXEfeXPgpnGKvkVodrcIExl0swh0kSI1WQZtF9eExfQjoUOvYq9fmWUX+8yVpPKOKu2vqQ'
        b'SF06yH2Nb8/AGR4xLBrQOdt6zw3SQ9K10VZHfisysnpQbPRlxHhw5kZUzlKfAF/4GIL67kfO7r58++KF33rX+bMN'
        b'UEAXx9rLwpzD2EHGFUEwgbiCpfUjWy/oGEfRHivSMhy2ANMttPLETLx9t//KxXP2009dYxyTWWYdB4igZbS44ppJ'
        b'kd4cQ1ssmt/WkjvkzPFCFQtR9mHY2iCUMfxRjNSfb3T07JWvC/Pri3NPNIS6w8Jhyyp3uHGVyTz18wBWCbUcfRyZ'
        b'XMUk1kQoMeu9NohATtDXwNCCvVgZcjm8JX1oxYUcFuPVqf7Zx2vizdvapsbO507focGMexv0WFG7vYlQ9mn6Npz2'
        b'djupiTzl8Fjyv3clAQcmpBb9qMmafJirJObnH7Ol7cGdcXvb+sJv3FASIOlXm3w0vndbu6l9ucOZ1I1jJAF67LUm'
        b'4Q6w69mY9lwPrW1Ce9LxJC09uVMf0oFkz5+1I45G5DKCdfq5QstxJgcUEUSZDDDxgyPVDA2DtzbjmsnOdbTDaCNf'
        b'ddCfe0yW3rv16v0t8e7cHZdD+k1/mcmPA2lyWJcJ62lLKMuPcmQC7IijFNiUOToUMJvievTGwW1ta2YgK1K8yls+'
        b'+ZgM7djXt4F/7/uY85pCtQvRAi8pc7VjrWEAAOMm4T0Bo4YcAEe9yWYgXXyDszRzjIHTXoX5/uUYiFH35537D+5v'
        b'ExfTexutjeZ+Xbrcv6x8vCGecikPL4P/Qz8AiPZhpqnOzGjTMWcJGWYdnONPFd9F+0XtuCBdgwlbD2rtmEhMGdRf'
        b'Xyk+vdnoe22dbRp1Ey1wMurZw1T/XLymuO+qKjMLdV28PSBUuCmhA9FvkbG1g0yEC4CvGVQFTuFeBwB1RE8Ky1fj'
        b'/hNfjpFnr96b9x9OPDN94/T2vavjuwfx9z6Gfb77JsnfnEFXkyNbrXDmWROZPJayDlcEAosAZ9rBJxTukQh6eFe8'
        b'Skb+/GNM8K+crjfJ1r9UENnIlFW0FGtWyyRJyOmC/6WmckEfGt9ga3UIqsT7oGxmhBjONeIaiMEnxEVE/AT2N/+g'
        b'zb/zjvNnJwke5VRTQi9hP3k9kJNgo6pRQHa1mA/hO8lUlPtoFd7PV4c0Wbtq83jMNl9Wfn2R1b//1PmzcLnWS55S'
        b'/qMdZsWQYlzp0LaW9lMB0iP51jKODaYdkPHQSFfw067Cjzes/rGvFK9eX175K98Afop0LsNv66GQjgDNyNwDokaz'
        b'OwiUmWB4iCzpQo9j+gPWHGYelZx/xZ4XHhN+vnx3bPM+8b7zZ9HFzU+o7zTNI+pV1TEZI5I3xD548iJxHhxYpRpd'
        b'kvtKI5oDvo9DtKss5Ee/UvPG5Wjd/vtw9jpq67MGWHgFuI94kBiJiTWbh/02j9RXuSEAX/lqDwH1nMqAKQfzJpX8'
        b'41/FaK2TF7dJmdCt5N4FEPNru9jO1FpTROfWnlSTVY3pjBkac7bkyIGEx8jwpaGa1TclwD/9lZr0B9Hva/5n0K/A'
        b'HqO2bSJKMa/umTCznMi4D8nbfEDUjqMGe+D3Ry1QuMJXC8wiuqsp5GNfqV0vzzMm7KxerhgMMCP4Cahl+Cl0Zhib'
        b'oS25G8SBOepQTrYuzXZkEhrcto1RVclkuzYL21UC9jOPmYCRRbaZz+P17gioqwbuEnLGe+PhB7MO3Aw12BDkBKgP'
        b'5L7Y8KsJv1F8TNXfpJyujl38Q818Zd4Zd09vnCJN7o92xduf+8zLW6ccpvgKbepjlhQtuX7YeZSmaqucawlduinM'
        b'TqbwFnkNn63MJ7AiAftHDr8vacjNdhf+PC/R4AM/qNoPV1D30TBtWpbCvUHMGdJKvSWrkgCiP0JIcm6xQWInAGqP'
        b'sUinuVwdl49+ueY8tOWmDnJsg/7F+8EDAopJ0Z7r4WH3FnlG2A2tQoNKrrcMigMSbVk7snY6O4zp6N0FXC7/kdH8'
        b'S4/PydnlfuW2529+5vzZ7D0KcY6sQqLFdJmsPd/MkJEjj3AcELpWMhRjpOHbKjXHpmqUYvih8lXO1917eyN2O9DF'
        b'33/H1mboW0h4UJ1z07ryjAyFXVMiV8WNiy+YwuAIueFytcmRfT2Y1zfpiJ/4Cgy6d3Lnjvbyb96/fdIvRcP4GrJK'
        b'sqEkAx4C0Cu51U0aLTysx8uVrFMP1Fc/evFQhyGAignPD7Yf/upi3LNfrlX97u0HL9+5uerLJ7cv9dbHv5OJi8xB'
        b'DgdsK1skCkOCq9saKpi5CDDwfWhbvBdH/s2ecarwGHQ4yDCv7nL8yS/XorVObj44O7m9jfn+ZxRmUL+pgxyefEHW'
        b'ddkDP4cPeHliOiHPvdY5HVKf/1uZfBMj+i8PaI19QrsColNSA6eQKl7iV7e9/9+3I1YLeh/O10wPwwaYdShhIKV7'
        b'TKpNdJLa08TZtKRSkFs2NBUE8l+tBF2x98UnxV6/yPDfgsK6VlQXmAmNvFSfYUssA0jDARn6WjN8Z9rsUw0AS661'
        b'8n4eCGH0j3qV3Zw8OQr7Rab/M3gsAENe7wblHaOWptoysO7RG0kU2Tgb2ap7mIabCDLIEOm1RpiK6nQO/0fZ3fgq'
        b'loduqpjnEoi+///ei0RrYEdWPVgIMO/urC38RwTSGmtSMopBGyEDLs6InLSr66gPgx7cVZ/+wT/U3qvrkT/0lCC5'
        b'FNXPhtUseUDbwsdqcGtoWjaH8xkqBnk1x+whtYRUhPa7YaD9UWWqX94vvlxlPD6ndw4qgp6CLtW+++QH77Xrdz3k'
        b'wo6ZTCpR6hOGgYRGT3dhdBa3TeHqO3/4rX71R148Obt19+6v3L9x73SfNnr55Ozi+d/WUZc108HImnx4khAvCOEs'
        b'iEUIFWzUwZxthMAMn8oRNBzwLqMatIWES3+UdY+vEETuzM+efbFX/+73qJB8TXIo80TOVgE0U4E8t5CfbMzCgbu3'
        b'0S9vbCkzDG8DkjINOOQkeVyF4dtPCkm+tPX/MXBiMzPXU+oVfRmZx0ZYMuMuT4d+y9b6pnK34leea4q7hIpwSd7M'
        b'NMdVHLzz5ODkS9v/L57SGTuwIRdVdZJ/C0nIt0GyKVPHpKrNFSBf8Drc2R9+8B5LJzm0CDHHeFKKXkaf3Tp5uKX3'
        b'Jqt/9E8QZbWsVTDJjRB7tHD3hBOlFXLD/gm6o8EM87EA9NSGZiBA2bPNidx0xeqXn5TPvIX5/4mchqEdDSeP3ZKE'
        b'YKUzTDsIgZwnoSBR6V01Xg4FgkhfwnNxohpR2/YJ7vldcZq3eIF/idccKc3itDkxBjhyBKhAqgtFvjwMOAfMtQi9'
        b'iJQCcuEqFoXH95MMtP925QXmY/aadffB6dmtLzb76R9CH0fVUGS7dMA41NSt0UmAjPRFHtsDjVd8zjpZ6h05Kan+'
        b'pgUY6kjG+vIEg/WR27yV/f+p6nJCQ/LBWqAubWI4whSxP3qAGiJMxQJsWZUozsG6oxtVcIL0og3khSv233tyfvNW'
        b'b/DaU9rtBstVlVM7UohRTgeiIDsypO1Nx0Mbs1HXIGUNhxT3jsBNLvK30I1/gnCzTtaXsvq/vs64L9y5ADRjWtPR'
        b'BC6pjHbF2lZTHs/HIDvBa9DAVSv2o5N8SxwGiVP92wE3b2G+4CbmymiWhLRxeESZHYeGMhIFweUVoboQoRJ9iDUF'
        b'PAYKrKXJaarX+YO3B27e4gUu4WY4bR+6kOBwKCGd2bTOoHvSIlMdybgF0uhcUpH6blbF9fwbQjuU/gS95v7JZ7+U'
        b'1f+2PX92eZCvM+bucEchWnVUtk2HbVhL/gT6TfQ4lUP9oouTCtWGrzDFBml4O7zmLcyX1+isV6pTsIhAzkL1km2A'
        b'Tqbhkc0QZRWtQdfCnKtVROBaKRjbtXrV1tvjNW/xAvKa1Rxes3zveVSLDqrQYZ8T1BfzolXLhYXIQ/qboJYBh9EJ'
        b'VsdrYuibqNnNx+01l9XDbxj83O/8Uy3dTOBuKg+h8hI5fwr8iveYf0AfEREwSpgDomHB51WeD8yTee2RDdr6CY75'
        b'Gy4zX+Fnv8SY/x5O04IxNmqHci+FjdxJR3X53I41ya4EBMw4MjGwZTO6w11m1TKHDzDjq1stf/EJOs1bvcK36hXi'
        b'wIuliUDNUolI7WatQooVq3TtqI23cIdxWvOAK4cVBk5lO5OU89tBEubJi7e+lP3/zTu0rDVtj6IF2py1DK4dOpRl'
        b'gj9qnaZFgnotg3CZKtbphuQL2TeD1HC8acH2CZKEt3qD/5fA7SbC4L0ZCCsd0+4rZ/A+guurI61XUMUTjjWC0Vkd'
        b'H+BFKR6bLsPw3w7gvHPyJR3o7zEBsC3TmqyszmZve3cF0nm0IczE/c1U4ZE9gCFow4gMfc9R7UvU4eR4e4DzLV7g'
        b'/9nLJwBgw8VnbdBgFeuN1tvQGT/fta3W/GRORnMzEwgFIVuR6DXv1hRvy/ifzbccf1P84YwL3rTRfbZVqx8w/ENb'
        b'gv0Y8J+sbbqjh1zUbqPuI4OxJ5Ny9/NtUlfzLcc/krFcC5KxFfcpTp0TcBLLlxYa107rTYdXBnUPimbXCvA6gE+z'
        b'2cYnuET5aPxP5/2TwT/f3B1Ytun/hLFPzbbkcjq8zgaqN1FNh/O7dPlwATEbivaMB2xHqFqhRsRtVGgcsOsrpv/K'
        b'kxv7L2H818lxbNOJcaSTyR6fd6M2IFEasfegflNaYVBxaNCWbzGrDa1n8n0R+Lm6rXvriY97K+5yj/Ufiav1dWBO'
        b'ihOuUOz0y1WX1dMEzpn5cwfyS1tuaXMhJPJxJhuQnXsiNq5Czktvw7A/sv29gku1ESoEbD5Km/CxapoLLXRGNxZn'
        b'LXyhxhkn05JM736looY+ppjOO9m3ddQn4vXRqCt51jbysYxaeMWAdTHl1FsldsfoBLAGmTnQdk3FyY8Aqy4xeRUg'
        b'v82j/sj2926QJPdgWk1A4MjIpHgk9W2aDpmbrPYyYZRD1bB+Hi5mU3nThAAO2p5/W0cdzMiPRj3aY87gU9cZgxLJ'
        b'tkio6KxvLnZTUoXMDEA9YPWMfrRS4KJVR+mddqzf5lF/ZLtGvadMcgUxkqm5xGVW8N2jUsZYKxzZlo6yjV2dAY7j'
        b'sDZa5BRcbu5iQPe2jjp50z0add4iBjSHZKuCta8IJxsLjaI9nCmi7FAlDU48s7R3T8UUdYDzdr2pKOjtGPVHtmvU'
        b'17IBoqLdyRhbLq0s67N3EQppkgdlqnUpOYSWWj3NifpCph9NKFpzeltHfbXmH416iuvIKzvheB6Ha+pfo/MLvMlh'
        b'BT0I2up7XknFgSrTYLAX72R54fh2+/oj2zeuZ3J6QhvZEg7VkLVDNePNQR7tKgbJZBGLflkc6XDaEnMxZuSgc6O4'
        b't3fUE4P8aNQP4KRbAq64OZRljkOteQ4i1bRU1cbKzGBSQD5pZcdCxhre3tWxbb1p+/DtGPVHtmvUbU3Jx9Dbsu3w'
        b'c6p1VIXPZF4EBr/2ollN2a4eIGrZqHi7WOcXJAyp+/ZmUzzjDV/XolLtowctYmvD9qjq8BgJ1YkKd3ZCvYZT5mo6'
        b'jAq2L7gXDBNSfOS3O5u+bvvOpt61rh3u5rKqCcJMNYUsWjhtg0UqVlWcmGMidxlUeVRvU+ZpqCnVtv1MbQZ1yPtK'
        b'l83H2yLijTPb13Vm+/r+bY9aRPz+n9k9WFHfsK1DFY69mjK1I48O0dZ4QzaRhpgY1JKqfIPqQ4+RfUcEZqeTVLsF'
        b'4ay357h5dsoEPHyRu4+318XrLWbf6Miym/3u3/j6Cz33u+986tpuVILxs+ZhUR7hmL06Y9Mg+8IfQo1QsXqov1pd'
        b'KhioeB4vFDs0f6nNklGjkgd3Hr7UZRvji+f+vf/1/dfOv/nSuIdntm4+7CT6d/z1a+ffJRtP3viJmy/ja7yPnO/i'
        b'uWv9m649OP8WNPjpyZ37J303urh7R4558aE/9W3vvHbt156pD85uMVtnrz7zI7/2zP7oZ37E5/ThZ3SA8Jkfeeax'
        b'juiY+7tl3fVHv/j6G6019qA+8+FnLtuY8tsPorNGGPvqFsXNgEWFrTPdAJFa627dRm0n6KxA0Q5+8wmKf5Rj9BWe'
        b'+fUPP6OjBw9enmO/4D4M88yPBO3bZmPch585ufPK3b5PIPDm9sPP3Ll759F3uQgjOg4MmnzK0NAY8+FnHtw5OePP'
        b'/N7Lj1f7AgYfe/vdW3O3CH3F8xpXXvfK6FpvzZMf3l05c/1Tn/7kcx//mNoGX//EJz9z/cc/9amf+smP/8QXDXPQ'
        b'prAvAQZbB4lmhdQcnBFILyKGzmu9Ou+To+Ewar+3UCCSqlDLZTTMe7WT1/tFZ8OHzYfNL3/4mYdHka++OzHxR3j3'
        b'L7e585ve5ats7/zMr//6ey+e/4fPX3v+2ufVwVrde98prHzpr73j2rUd7i99gT99/vXgP3/nvLsu/ur5e8bUsZFx'
        b'8dfU6Oml3+J7bn23ujR/z+cv2zW/9Nt86a++9Lf0fDB3N/PbJ2obvjsEv++KI148//Hzp7eXXTz3Y8+849r5ex46'
        b'4MUL/9unzr/2kYtefOHp9//Z//I/eMf5ux/9/XPffO9vf/DB+Tc87Hl8843f8NLv8Iuf//hL/6FeQ5/60t/hT3zg'
        b'S/8R/+WT/tuP/u13vPR3958ffso3XfGom6eqk8W0p86/8+WTF0+3pbtoVulKp/p3c5wXrn3u/Ou2L7z+q1/4/aee'
        b'f9fz7/r8xfn3rge3yWn7Hx+GzZt+9rnn/8efA9OufNcXffxzfPy1Bx/SxtEBQbLDuj7bQFzWWlvqRZ2Me6k1pGGP'
        b'MazOYJFJjjQ6OFI6tKSk3tUjmR9sxeQZ3DTGq6fXAXA7Eg3EvVbI7dQxDdJOQg32mbpdDmm6kCc+tclHoPZGOCI/'
        b'jLuJ2nRfdTo51+zSkVCHpGO1qQw6HSJcqrZNe+CPgCykX0qzIWuWzl/huFP8dfamHa6BP0JUlxo3VHJ9QwYVQxbx'
        b'IogtaQv+0Is0H0qz/Mbu8HlCxeVdEKltyUhymdNru2xGdbDQebTVQu/D9JrVbK/qI8KunD1AVZ2oa+pgWnWg3aXY'
        b'zMH46Sheam0yeikmCxxnHXK3zvGCepHMRx9K3U502/JORUvqQO1MvqHa1HOM3DgDOq7DaRnw1kYJswfVWF3o5DSc'
        b'QHraFya1r0ymRGdYV4Ed7Xdje3Iqawph720OBtzpEJ5nhkbgIwhockFk/KMh99qya2uh0XypIBIY4Ek+KS1jDtQV'
        b'7TAzQ8f/jRpn00cUvxAJ3o5iST9km7KrZnQOqWmRSAsXU219nY0I1hZCKOpWbVFzFmb+0n9BEL3xOH92zN28QJ6V'
        b'tbI9dKZpgrIlH8ZXr36wNdqZHE7LzPGi6yDNobX4BpcxyheGJOBnwRl4qcoxSzOjl1U6GmGuuTcC5nIMGENttW83'
        b'VeG/Cs6s90Jb4MNrtdi1CznJrPOYOsHtCRg1Hp5NPT9JoCvovER3ntmK6rNIJtBHJBU9wnQKnDIjz9NM8GmSgQaD'
        b'/6JMuj9WhbaWIzIlh3q3Bmh0LH0eXq3ldXKuJL+35rIZUROrVipqRphzbzoYdWjTTlu/R60ZeX/I87vqxmWFheVG'
        b'A+1FUXctlxazKjqjw42tLTozox7LTvMzMDwmRzJYhGkhHdTOR0A8cxr8yqySPwRYZMwQ7G6I5qttIjrLMt+F4Bp+'
        b'+QP3J0CsOrmOFmXFTKoVhL2OtEbN6llpemGChllFvUoWozXUnXrWhJ8zTwUF6ityrkfBR2WEBvpILWMaY1fxkUN9'
        b'TI1p0YBhDlzCsyJDjvXMM6+1hDLOGuxUo/dg/DiGaWsVfjxYy/vgT4GAWxEgU1ndwkbnpwoxgoqNzPJN7p+6Yn/q'
        b'kMAcDH61vs2kg/YylflE+hBzUSd4CMTcmi3mcDodkpplhkIzXTOCL2Hm8Ew78hSWtpcrY1G35sW0lgkyNDVvWM0l'
        b'E0aE+088uajQzzs+ooivmSwM3qwvDW8mMaXjTgQHMOoyswK8z1LkpLbrMLAdKo4JUVaY5nWzxiIYcLaYBM0Gm4xL'
        b'Mzve2mnDCWfqOan8jZkgf8yGh4Hq2zuj3jvoGGkEupkZFYZm8osdVY0Swd7SVAR1WO38tMp8HwsOxBwfOM7F3tjq'
        b'R2y9qA1Qg3hMExw4VKqsrhboxukQzmpfWA+XiorWUXM6VsooKtg7wMd8o460MmcSWDFUVTNtN8MmAl+NO/EC4jiA'
        b'4zhL8XWZBX7YMuQX4KPRubkKIDAguqugdtAn6vT0OKavQOFYzHTyToniILhbJ1TGGksOHrQRunR+dfUK8qsaH1dY'
        b'WU0MAnIn2LDMTrRBfYPITVmL5TECInyi0gqeqp1hpZ+hXkSupRldJSfz58RQ4PZtqWNDZ9RSMfHwyNUhNDd5CXLE'
        b'/ksNREoc6qNNJgXywU2cOGU8bLrIKyRfvPTAUpl10jYn6F+trIi+BhjrtDaBSOSmYvlHT4pbE5uXItJPF6IaXge7'
        b'Tx9rBcDzesbbPRaItgWvAL09WY8pxEfI1XFhX58gmfpgEBsx7m9Vl/8x1LY4V19b3ym2qCyaWVZfm16Tzl5pedwP'
        b'Em+Jfh7YwcdPXFgstkIQktEgDdBYfjF2/fbk7eEfS1V5kF7UflZPDqvCZh3urmXf2WGSC/vEnZlgkbPMkGJkeK87'
        b'PNJKquZntBov70bT1xMDAAepy7rohnqVMywZ7GNwdzbaeQQTqlV/JhIZGX5hk99n2BoDoo1DkiumHS0KNoMjGGA0'
        b'h1ehr1oK70jlC324YNRzCviGVhmv07tqQd0xDspiQ9BtOA1Hc7pmZHcoWDi0UULsbjotU6LQKrykdAJFV0N4RgcH'
        b'Vp/o1IlTMTDSqQvagCVu+qGKxi7XWjgk8YULdohB6Noc0kZzOAhrtXiaWB5DXLt/0j5ZEHV/AyaTkJaGs++SazBZ'
        b'Uc1vy4lpOcQipw4E8JI42BHmIscWqxp9rzPiXRe2OBxaSQBgHOrFhiODQ/GQQ+JpaK48E3wKD3IOtkPozTyA0jKd'
        b'DsNqRcpvGuqWlqkiUBBC1VG1QsDDJ9UXNIUeSSbqSBCmlt/SnhT1oyIACMO5dmYfWBWHDhF4r2PtgbzHKDR0HuTX'
        b'ic827ytkLvOxUZNqLtvg6tKAtRG8dQNuqhMyTgCgks7CzMxv3gezxmBuMvMGbu1rSRD0DCBpC+baNKmxZ5yJlEai'
        b'hnx22FwQBQb5GcWljuoD/guFgGYYpSvEWxBU8I0A6Z6RiFEHEi8wPgz8cGuTGchEDAY2ojbXJKWUjS4+8ZiIs/Id'
        b'Y++LaCygnF1bmPtEx95HLmo9rhslQPxdTg2VHzp0rgPKSOao8Ne57xY3UXJhTlIMuF7IvlYr1CAK4A2d8uq0gt+S'
        b'6nVypAi8oDLqzAVVIXLV3I6cykwQVkllkrCMrmpDUg2RD9l1TpVYTudI4z4mCcPR3Qh8ks5D4SACvoSvq9o+wykL'
        b'zHsYBwMRYYdwHzhmJ0UxzJkZsDqv1MHMceBL6lwS5J29VYAE7Agdvhj02yPYhY7G18cBNwt8Qcdf4Lhxwe0ArWG1'
        b'NyCFveUJwQv5qF1YPYcuGqkuk9y8ekpMncNDvFWzPxQcrNpAboTQaM2tsNMywwLjVDVjRRvAuclyGF2Faih8sgf6'
        b'XrtF6qBkmDfSIOwWBxAlkhV87/K4f5T08FE1+kGAV9UVEQTQsQQCEATVqeUeyK4eNJTqGHysxsI6eC1cTvU1JeRK'
        b'Pj5URay7O8jfOooNawF9994xCMpw4ACka94aAiDICQOdOGJZeBjhKD2IxKywd0gMIGOVfpw2SM2x5ySSPQHtmmAl'
        b'2ztXZHiN0m/J6olwIC4XaJF2H4cCBqyk+uFWveACDh+gPMlZ9fgvU2HWRI6a5o8v4xNJLX74FKNTJIb8g5Yw+mTR'
        b'PRxQxTn9IK+C1Ye6Q+7lzqQzuF03MsTBn5AceHCII6Biq2K+KTccFcdGt0XlmCRaqCK3Heyk+Z7UGUVN3i97qMHW'
        b'bBPES5UjSnl3SBDc+tDR9woiEfMEvI5z6SOMLrpC3+ro66HGYMpcoLyGjwSzAAGvPjdJNFiKWWfClhKmJMaWrU2d'
        b'0cOyukwCKeCzyqmjiL6pYOPU9ikyaRjL302OSBlG1uu4AebrRYIKbTMhlaOb5F6SsFpTjSgWB65kZR5LjgCE4MT7'
        b'AhWbmRSADP4sB2eEeAn5koMqA0MAoa5mWgpMYKQzbjoxCrdAL4VYYFyoBvJaUo/HzcFdxf7I0ByhQJEBuLaPvet0'
        b'B9Q9EP2kMK2O4uUEH+JWjdZ1ANQBA3ItZKklzQL1xEokEGB8jB9eg9QOahqiFs7kkYWgRqVDsJXlIhps5R3s+DYo'
        b'dKAmYEHe5wmvAJyJea22paEeTgYb5pDqwS3IQiEJpZAwzo09IxnFX6fUE7o0GwN95IX6jk7mt1bVLi3IWpPmAy8H'
        b'34OEy3zKlnfwbfVU08JJmXYGUhrkFUgYhyaHyQXyCbXqps58HVX92nmDRk4OUR1mdQ6y6MRj5keLDp82BE1Q33nQ'
        b'jBy21HEEpYMyQcwFcACuP6A/Jkg+mk1RCAZSrpquLhVN8YfMxAz1ADpU6L/G8AfzSvQSpz5jUUcHqtESmCMHb/Ah'
        b'Jq2Jv8POdmVfAXh6B8TwWllLpgU2kxOvHx5GaDePxth88dI/1zLEP9/LEKAQLFenxVzdDTVAnSK7md0MF02kBZQS'
        b'E6r/X+pIAJxbkNfyk0UeokoNxLZOP/HdZmU1wMArbOhFhxMKHNG5bp0OqjVplcSE5aGjL9mVuJkGGKPD4Yv5Nh4O'
        b'AO0Lkux8WYimQ20M9qFDjrwW9hHSXaccSr8kK/DNvBtU4frMpdrwp+51AjHXNiELADJIqlP5aPpdywMxNLi8w6X2'
        b'Io9nzIZKwqZgKk7LT0Sd20JuMkn7GqaVq1B0tLVv8SEBos+GiNFlgoZuFmhX07oK4Mdvmv6QChLxQtborGlSi9CC'
        b'EzMUCNBsdP6hZaeFYl5Et5hBBeLUKSaSiRaBErnL6HcdOsRvsdORK1VTCwFl9IG3OXsSi1HAGT+XtpGa0/UOqhVS'
        b'OkwaVJi+5sVgqQXsJ4oFYcqEmgFeRkec7nTgcmxa40BbTG0/2wmMDSQMXFfrlxPINLx/0UVAKoVVq5tat/iEGWhS'
        b'AZNDkYA2RkYxtBBe8uaIDarKN3u7xLjwEQV6PiDZRzWHeuEw5xpO1I8WZbTXOpEKDbxwkK5NbCaJAYjAfcHIQQSq'
        b'qbyRAjJC2VCgV0pKrh5qCYlnMquM21LXU50uEKYVMftctJxAFKLb1JcP+bAqTJJ0feRLNu50SihX6BwObdTDU+GI'
        b'S8AItMmHrIKXTJ26JZCNLqk78qEFu7wXAEgAZM+kQ8e6IwZeDq7x+TPqfDSjpMUBeJXqt6LAu8GFtULQjPZAlZQA'
        b'GSdQxD0i6gAqrDNABS7RmdRDOsrBlfBxaP1CODtiTS1EdBSkBlmBxmV+fW54JkJ2uaqVFkUu6gyshP5lLXm7oDav'
        b'RademxbEEQBh6loNrEASwbSAtlrVJKUdbjhGAORLXldIIKOGgBqQX1qs00VhuKjTnYx96yMUsKmHLsVjpNRdDOTD'
        b'/qbQZg6N7ngJWi7xNkiGq5Rc6xMwJjd6FmWCuBdFklcoG+gzEcbEE1q6Qg99wWTyKz0JHXZmBwlMrV+gJ4w7brut'
        b'iL0wVITVgB97rZfaRuDBZPGiKPoa1U5watufkTIMJ0wE7EdyHzs1wlHIVKD1wv14SfwGjSl/Lto4g1IQuzAOg9OT'
        b'bEVBYVCgmZf7bCuSrk9zvDfjB6a7BcskKcsJ0HmQ7rJWaGpyOLJaWzlVzRmT1fkAR1WMMHG6lCurz/RUwXogqPEh'
        b'JDxqmMzE0LeqsG9Tnb7sgVzT+utSM7K4Y0QdM0EZqBlBErRwwF/wDKsNeAcmqm4ZtY/21i1vqL8MA0SillFs2PsN'
        b'fcbECEwdzbES7ww4Ya9gJN9CyCAxKCwdvlc+sMVNoMFb1b6GnV11/HGoVaaOvMGMjK4UBdbJOgHao4NvkAfvutaq'
        b'oRC6vRKAh8Bj9Y4RaJgUKFOsZnWTaV/ZMPctixLCyPB3XnGp89AMhG93vmj7EiqCKXItp0bNTquJTQ2P1bqdkQnj'
        b'ckWUz0c8w6CyBK765RL3qGSyz9K5pL19E1ArgDv+yTs27dlcrqkRWH5GrbgOuCQEgh8M8FA86CAM0sLEMjfTgO4j'
        b'H5sHm8SsQF58YYG+Xrs8igmgMUdcojkVk/POOv4f0Dd+95pR+yOpEjT9lHxQjuG3EEgL9hXmngfi46hde96IiSEt'
        b'TWxUQ0BfEnoICuxBN2t1tSZqapwr3gL99OIY6uJi1KxSzJBxSYQ63CdqFXKtPSPijYlUQoyqVI9ftRhFAhzw0fJU'
        b'Vs/GypSAjAjJDnnXHSBo6DW1V6QXiUK4PKVnne6gQ8dXqIAus2HUDUBLItcqG7QNLw1Zl0NBNIvOW00lAV5WdTyB'
        b'7EJCavMQacN6ZhI5kqbaQ0zFUEwH/4BytEU7IW53oZ+bxFajjnjWI47VhBLBon7DCCBQ4qhKnkVnfLuU01wF2YSK'
        b'PHRMX326LpchCM9Z7abtHTlkQOupfKMrTKGCSaRNK8NZuyu6aNarWaBWTnVT3KXotalAkjxWRfxb+1o6yloJhaL1'
        b'YB03jzpErL4PPa+pxOSHmrrVo26VBka2odNnPgXdokckjBo8Ij8iu2HTKEGtz88Dk4K/bJaru9SyszubXV6+yjhV'
        b'lRbXrIZ8vErnpVBF+LzvB8Z3MSCtyODdJCtQdFODy6U6kSfoLV4eu/aaakhoUl5E7RtBu6ge+aVF7b/iO2r5mLYf'
        b'LujtXnM8tNxijSe/aRDgJh6irCbau+kBRNxbHToCengpHZw1wjZ+o+juHgviTWONEoN2qiNwDExLaEW7mGm5DvUX'
        b'i9P5bS3ya02FgNXFWLWbsheSmy4BADt2U2d8PaggxYPaVR1TOk4M89Kp3CjCi7D1unCZmEHwtWb2cGohHT0L6Ziq'
        b'QifsjHpyxzKVkStaD/zBeqPVUQi+V8+QIE6pmlGNBb+TLJ60w2Qg4Nq44V1tOyxc6QAND9D8MDAFFyQSjNp1dLhK'
        b'90qWW5mA/pg519QtpUlHhAAw8hU0UksmU4zBBVAICqEtGVXeqrch3hz3ap9yD5gJN9TajqSwanTRyIAsRKgccDXd'
        b'07RQox3ReojjJxK0yCvcRTMCI1CGrkX9jvi1oWu9kNRJIMBCfegq2ifNHsyVNn5xH9LUGOCJuuSQR8AhHWdcDI5q'
        b'VVQyzA9q5wNdX1VIjt6qRmT/YA50l6EWwtzRybZtr6e4An2CpMBjCHD13EvSPCS0UvzQGgIcHfjk83F+nAdM2Yf1'
        b'CAAk/f4IxClIBkWYBKtaecPpIGwI2aJRTEhGYCjgwFXL98qXDCeyaOFCsqKDgbxXO5AXQAhMNaITCuirRpIJzN2F'
        b'zowcnAWvLBFNFsHmHi2kZu+s4kVSINoWzhCaPnUdOJE9eSnwRwlz6OJJm3UZHaqYVAaqgNHKB/ILXTCle0Kgxgep'
        b'ujLhEyZgbULne1CyRxXiYRtQLM6lXQRI6ez8/CxKAsi+WBah7BrJA2DvuqxjgmIWSaBu004brEbrq1DtpP7z2mxg'
        b'ShiNfXm1dtZ2uashI0HgMyGXEOJSTmrCYUFu2ed1m8IBCdAFtsFMvpIPlIW8s2rZitTMlAHRMdmeKiOjiyW04EAC'
        b'ihkIQUdXXTkIxVHirtCzoHsl9SLoRSC7t9KchaqQKonEovPsM5HPYS6MMY6iOx9myfto9VDq7aKqe9coK9kzftBN'
        b'iECBO6i8VIcMjXZEpLxD1D13ZF5IZc/EUQfESQIrjL0w5HQns8CQkVTbAj5JtwzOAYsBm0SugtJlUo9FzPTxQHVY'
        b'q/ryy21RwgebdYyd9Jm15EiwqUKaTKxT7SBZhGpJ2te9M6Tr08iuhGzS0XYxvjUbEiDrOrnoyO3qWO51ZQCOstSQ'
        b'GSlR1IxVRybUrQ9NrONAdakyQ1YwrMB06JtXETxwXNUEo3eQcRavNLqIAFlDkKDcSXoF105uuqK63L43qn1ihqv2'
        b'N0ZLxLe+7TCq7mCSERlDN94g/6EYorhqU4FESQQs0L/rXPzePjCZF4j76NUB3MJNzN6FbQLDemzC13WlNIEHzZm6'
        b'5S8fYy/haiee0bXNo01Lg44RAfyOPCr/jVPV/1h8qBs5OYdYAwhdIDXCVOxeGGoS5VGLjLvzOjHIPx5MPZMHkE0r'
        b'YSdqWtGqZa81qUnIItcnMozwIvDWiE60cpxCKX4eeBKD1hKEYeojb5mqbsM7LtcSwB9QQy0mXbpc4cKGoIwO3AYo'
        b'01BerroxhnyqzaTGFHTDpEtaDN5BlzCiSoyU6haZ2gjuKuweK+vqaHSQV10JGqZahHLIDjBkPnSIoMmBkCYRkTI0'
        b'AnWvApfOy+kehwGtUH+L6HqEH+jiO3Kl0Sk40XTIgCeKJJx0ebbWiMClvRADc4YfQ6jBbmP21eAqplIdw8p720rN'
        b'67UkhObaB9ttVk9ZKBpgqY8gV6NrgwoCrIqFEt+odXA8xMMU+d+sy1xR2cjySuyDOepg4bUhmzfvbMkG3eeHch2j'
        b'Lh14ItK1At9VggahhIxrx3uolBOJDIXv5EkYEblpr/Zp51ZHAxs5C6mCq2UrjTr3Bg2jCPEnvXVpzSTdgTisa6lU'
        b'6oBdCzvVHUe3JZHLkLdMKJDfRkSY8QZEtL28bRw/J5cS+2ARakMP3ftxWZUykH25aley4P0o4AG2jX3/u4qUdElF'
        b'JmySejWQOrt287LOMWhBTMNJmrUNKqAqhmNVj4Y6DnWr1DLWxLG0XYBZoKfaCw7tGZLYRIlUv7V3SnQtSdeKNgGB'
        b'S2FpVDF4gclHmJe2DHSYM8Ay8xSlX07NyMh3AZD3Oy3rnE1fE+4OvVaL67gPRatdwNzsyuteVl2UhHzwkBTyW0Jw'
        b'FAGc2Qsx6iEQia6lnqxafQjS9jrqPplrhz3wLjX6U99SmKlRjY0OoFT1TpcVKJ2GCmNATcM7dDMwwZin9vq1Ra41'
        b'tQLSkCsdYaPyBTXz4vtgdrvAj6BVzRx/c7Fbo5UzvkQS1dFWlU8tqaikXk/IkjmS1+U4uqlKa491L8R4SKAWu0no'
        b'bio5qoAXyUuY97QO39WHAK0JHKOrivZqnBgq6APR3LuJVrvLfhPNxizguGoU4fjIMHH30NRKLc2ickpoVJTybNrD'
        b'dUt1jRs75eDJZW9AgFp5nUkky9lL0aICQgj8UXKBpanzvG5PJkDElUAFxQhmBv7HMe9HZeBJlaJ3ulWdXzzRo4Uf'
        b'2LWKUZeCEmX4+zKAEDmqXC4TAp2wEl14oK28tS+U0SaJbmmXA4AZeqWyRcrsfCZBVrWal461d9AAIl2Va7KCq+E4'
        b'+zhmR/g3ZAdBwZSXoRclEqxbUKypDsi6y2IK+FQr0FWTiCIAkDYBRvTG0S3+DPeQ8o2porx3eRVOhPSBmePA2LaX'
        b'xkj7TjdPHLqQLGtptete1ADfaUmtvpNORPNnlfjwVhCGlFXYAtDBZOVaC0p+HEHVKHI9FUYfRVdEFS3uLVB/6piy'
        b'juYtHWzg3TzKlIwGC9wlJcSe1R4+ZAFwqgTfxPPjLhIF6ZGKykSHpJhWq3I1zKiRx2G133VCGhYPxVyWBE9+MRWm'
        b'7obKkpqWzJciD+8nCJU71K+BVGiQOaq82+RA1ShHPXbZXBAB7VknY2F/BpZgtL06ebepW5iEPglmALg6kmG1dm8r'
        b'OKlQbUcTKH4QQF6NWtAxA0KEuE6q+mrEN2iBx27cgWkbVQDiw3uZsBPKxWiVvta+jz/j88lo+ZMIIeQIqYLATp2p'
        b'Xl7nYAwcs6uR99w7aAfAfUDumBZy0vBaVCFPa4+2bqEyNNvoh3A4NXK1Whfp1ROpMFa3dkIkx2npBlasa7Lx+7gP'
        b'33ZIPN6lstGmlTtPhtRVMBgGNJEzZy++i8br9sLmjNAQhnUEQrSpu3hVRlIZC+kpqn8WL0/CqVp012r11Fr62mHm'
        b'dJVbVXGXWnQiiwkeEDjog1F6TV3mdLxDBc0HJFZX96gERpdnwaq3d+L7VYUxiEDtBmPMRikQF6hk7ACjWaJSAojl'
        b'ndrvGLXQJnHDSXYSIMkuHUAnZaP9Rf5i0fKZqvA67qJdBgwkqUBiXNDp/Ig4Jk6HlJaSgGpPSMtSjHuFoFfoLbS3'
        b'E0bkWdhEFRsLuyc0n6xO2hEw1/qWvVwO0g01OgM6j6xenaFJ8TpV5zKdWrYnH1iVvTFJBJrpeex9B2CefB8v13L8'
        b'0v+QnCBuePbQpsRUiQr5S+XvEJ+sq4NbUE9WhK06ZkMqG0GgbAZRNP1QrSDOl6UDWts9Ebwa6nX1KIVIQT/hokCQ'
        b'1bqS0x4UlIzw3NVKQVNEpJK9mCXNJGlYK4JL90Uh5VrdwihMXgJ+Q4D5vR8+CLu93skASTskMSASBNRAm/UQMhJh'
        b'Vil8GCJJ0YktqNaw2N1ctQWp3bRnRKg5kAyQdgvnAA+9VjKGOtAji5v2mICHiY8ztii4AQPYqVWscu+b+S1hcG1e'
        b'CcyRMmXMShPdRZvoEgLme098Eb/VSV0PDQO1CyR0q2XIqvR40zU04Iw12s/fpV6LgFXd8E4PWo+f/GivyIqgHVo1'
        b'nNuFcEkNxOBbC6pyHA4JpgPwJMLZq4i1WjWJ0Bl1bFIq1p2j+xLsTOjsosAtPHXNPHQNB9AdCQVtzIchh4QMgRd3'
        b'Bs8FeoEtp1uKoEVWe0y7oBpeiGoDuNTJP4mmQgmTClxQslOLEJABq7Ie9aQ/yMT7MCrolERpthJQhaoKV1CoQxfL'
        b'eX5xx0cBL5HsynyohkhlTxDoCVIEj9Ihh0Ie52UtXU26k8qWTI5SdTw5lA9K0vxeDW4M3pvVA53ZQOejBJwW0xyy'
        b'9qi76pUZVLF9KkX18HgPn2G1xjctSnX3MiF0UiKjZRhQcng7Slpl94zw2NipapUcTBuqfWTidHULo1Py3qfTClAk'
        b'lRc1PoTiTkR596py79pCFfARHAzWVNeUDSppqAQXYnuEoLL4lPFs1Q8lpdSqAgfVm8Ts67Eu9Ugh4Rq75HFeV4WR'
        b'kLooGSC0C8h1TYZiIfI9LThxPi1CH54IhI9vK4a6QKj4UI04islQ8qgTHsydegOj87UdkoFhXtN1vY7qfdCd1tvN'
        b'L4BRJv/QWmuG0JVF5lDjgAZx05Xe2ugijp2qerWfkdU3tKvY/dD9LWFX6FQt+k/ym2rDXa+qup/KITpP7qV7Ufsw'
        b'RBNhCRrSJhxi9npmSLcV8Dt0SsU3IVqZRMebaXFjqLlj1cp4sYduNXSQDSigWhsk0gPOP3YS0AYPPHKqsYcpAVkY'
        b'lnKsKjQcAKI+6KA+hBpoA8PhtoT4jGi8xA9pEXsU9d8lZqaugY0qkDF5Vo+4BCxU7RYEn9qa1C4hBMCpQaC2rXQd'
        b'Rrk4uzykeuu792GwW++99T06J3b+9OV5zhfaR8/fpYNxF+fPf3UH4+7VF+d1xvHyhOr5uy9Pxim+VHVjnTEDqLd7'
        b'nQW33pVkZKVIzEvpOoPuVbH6Ijx1aeVa0Qvi3cU+1/bgnTo09tGXXuG/j8lUe2nqS5/VRz6bAFyycSvqPqeTVTqC'
        b'ALwvQ45xOuSsnvlW7Qc6cnxXJuPXwPzY/UqvmPnHPvQ4zXRvMhNSHXXxqNUZXkce9DrmG3EP2K3XiqlT1aJ6PqUt'
        b'6iFtbpCirBZz+rp48PmLT5xdnL//yvHcW/X0xXnx3L9/bR/5U4P0fuviuWt/96lr59/88PTvowPkbxzf/pm3frs3'
        b'jofef3CP99xHQx+eJXWfIVwAql+4cfld109e1nvyzY8Oa//eb+hksxYPnUB1qDKNd0OUJNWhRal7cGv24XR8Dd1I'
        b'EKoJsk5UBEWqqrYvHpx/Te193jvT4cjzr3108pe/PH33dMxTwuA99+bpPi546+lbX3frW2598NZ33vreW9936wdu'
        b'/eCtH771Gy/9Oypj+rwef12Pcz3+Bo+ziwcf+f8BcC2GKg=='
    )),
    'compiled': (317401, 'de3430518c0781b5c64ce8599b4e8edb36ea0c98770803a259ba56c59528e9a9', (
        b'eNrUvU1vpsm2JXRbAgRXDWLQUs9TDFqifDI+9o6PO7xXSCDEx4ABUql1FJ+Vviczncd21qlCQuphD87wND+F/wAI'
        b'ECBAMGiGDPgTrPU8r512ZNr12q9dpT7n1q08zkw79hsRe60VsWPtf/av/Yv/9x/+g7/if/7rv/yTP/8bV+3d+FD+'
        b'8ud/72q8H+16dDFn7eLDp/P3o59d4bc+Xp+3s0/vy8ezH+1f/vxvfv54fv37836Fv/tvvTu/ur64/PlvzF++/Nre'
        b'+bW782t/59dy59d659fhzq/jnV+nO7/Of/nzX9/+LPzgf+eitc+Xl+NjG3/jjVu/4NcvyPoFXb8Q1i/E9Qtp/UJe'
        b'vvDVwKxdv7CO1K4jtetI7TpSu47UriO160jtOlK3jtStI3XrSN06UreO1K0jdetI3TpSt47UrSP160j9OlK/jtSv'
        b'I/XrSP06Ur+O1K8j9etI/TpSWUcq60hlHamsI5V1pLKOVNaRyjpSWUcq60h1HamuI9V1pLqOVNeR6jpSXUeq60h1'
        b'HamuIw3rSMM60rCONKwjDetIwzrSsI40rCMN60jDOtK4jjSuI43rSOM60riONK4jjetI4zrSuI40riNN60jTOtK0'
        b'jjStI03rSNM60rSONK0jTetI0zrSvI40ryPN60jzOtK8jjSvI83rSPM60ryONC8jFWPWL9j1C279gl+/IOsXdP1C'
        b'WL8Q1y+k9QvrSFeMkhWjZMUoWTFKVoySFaNkxShZMUpWjJIVo2TFKFkxSlaMkhWjZMUoWTFKVoySFaNkxShZMUpW'
        b'jJIVo2TFKFkxSlaMkhWjZMUoWTFKVoySFaNkxShZMUpWjJIVo2TFKFkxSlaMkhWjZMUoWTFKVoySFaNkxShZMUpW'
        b'jJIVo2TFKFkxSlaMkhWjZMUoWTFKVoySFaNkxShZMUpWjJIVo2TFKFkxSlaMkhWjZMUoWTFKVoySFaNkxShZMUpW'
        b'jJIVo2TFKFkxSlaMkhWjZMUoWTFKVoySFaNkxShZMUpWjJIVo2TFKFkxSlaMkhWjZMUoWTFKVoySFaN0xShdMUpX'
        b'jNIVo3TFKF0xSleM0hWjdMUoXTFKV4zSFaN0xShdMUpXjNIVo3TFKF0xSleM0hWjdMUoXTFKV4zSFaN0xShdMUpX'
        b'jNIVo3TFKF0xSleM0hWjdMUoXTFKV4zSFaN0xShdMUpXjNIVo3TFKF0xSleM0hWjdMUoXTFKV4zSFaN0xShdMUpX'
        b'jNIVo3TFKF0xSleM0hWjdMUoXTFKV4zSFaN0xShdMUpXjNIVo3TFKF0xSleM0hWjdMUoXTFKV4zSFaN0xShdMUpX'
        b'jNIVo3TFKF0xSleM0hWjdMUoXTFKV4zSFaN0xShdMUpXjNIVo3TFKF0xSleM0hWjdMUoXTFKV4zSFaN0xShdMUpX'
        b'jNIVo8KKUWHFqLBiVFgxKqwYFVaMCitGhRWjwopRYcWosGJUWDEqrBgVVowKK0aFFaPCilFhxaiwYlRYMSqsGBVW'
        b'jAorRoUVo8KKUWHFqLBiVFgxKqwYFVaMCitGhRWjwopRYcWosGJUWDEqrBgVVowKK0aFFaPCilFhxaiwYlRYMSqs'
        b'GBVWjAorRoUVo8KKUWHFqLBiVFgxKqwYFVaMCitGhRWjwopRYcWosGJUWDEqrBgVVowKK0aFFaPCilFhxaiwYlRY'
        b'MSqsGBVWjArAqOu//Plf553I1V/+yd/+/I//6q++/z5E+933b96+u/gw3s7Ln9+Wy+vzWdr11dsfL+fZ5cXnj92f'
        b'GVus2FrOnHFIITa8LVfXl+Xtx/HTtbizww3HmfldPf/45jtrTPKavnsTWrbNWpsaJhhMtgxtdVqfXTN9TnEpi83F'
        b'peAqFFqcrTbnRh2lQnsn++affvdm/HjeOfw33725vcDB1783j43688evhqxvZ62ib//4ubw/n+fj8qy8f3/2qbQ/'
        b'lB/G7/7+6gIDB2PFoJ04iSW0hHQaczbNSGitVaexJXE9DyM9NudHhzRN04YUEgjArFZbrsugP4wPdVz+DX742dXF'
        b'58s2zq7L+XuVPQr7olG8v/jhzXcBQ8Zn77HK2oTelAnd3t3wcxYTSwrDq6v4b7MtpWkMvqsW6dY3JB1bAAAhhnhs'
        b'GJyk793JYcyL9vlq9LN2gdXKi72bKeE6in7ICGOAUjtNzcuEgEMsrppSQPBaRR4PI5bcZ+pWWm/VpGRbRFIozR4b'
        b'i2Ms/hVi2SbGu4y99obLZjgxOcaZtM7QJDY7RUbxeSomzViTu9TsY5o9miAjtdG5l2bzemwwnsHIycFsv7r++auJ'
        b'iRmx1IEFU5K3YcwSc+xYTKkPk2fD17NMF1tO2U8w+VqQxCq2fwoNUzXwZ+axsQhj0VeIZd8x0OUIZozh7GgRyynN'
        b'YdJwWGQ924Lc5Wdv3YJOZVuNN35GyXPobCZ65C9bcg7HBsMZ/D78UtJ130i6+/d5e31x8f5sns/rdzffeZ5/LO/P'
        b'fpSbTSP63Rts7mgxyC5IBRpnddLFeFsislrLcahrDrm59oYx5pBr6rNiYlISsOSj8xjj/j6+Tjjb9NjssNIcZNps'
        b'TXou2dRkWvIhip0eoFJCMaXXZgdgRYsDptje4ogj1B5t1FGOjYZ57/v0zJVGeEIe/gpV5Ls3GmoGrHWPlOvq8BFy'
        b'ZySX3MCHnQoWoFUfzICUKILc5rCLjA/cLLmO6o4df+L48wuNf/v4JXqN371xLmFnM6XW1ESqHRg6QDL1mGdU7H7j'
        b'RrczTz9TFPU9SQ8eUZgakMCODSBvsGhOjKBennf+6/P5+5uEJRng3qN2i0hylRKrNdzrvffiJTqtuTeA35Bq8JF3'
        b'9a6NVCZ2Uamgvbabo5eR3dHdvmQY+2wY0EnMRu49i2IH9BDxPadPdY4qIxfsBmiFMnyYkn3G56+juaaCKPBjSgN+'
        b'Hh/HBu/WvUwc1+Pq+upmOiyAHclHMvayxl51jDpjFjdimCPFUn11E9slmS7Y0yFHrClNNTBDF+PTPBo/7Ibs1r9k'
        b'GNt0ZE5Gj+LnQIYdmgISKUhHdxGEypqgQ4q33jfTQKy8I0VE/rUWLLPUnJx1+egoNki3z8X0Vj5efDxvhXzxEjB4'
        b'y7AAFj2VXCPWiGASKrYF/tPHHFaTnaU0BeetSe1Aok2hxtFAUrrRVnr21fSjQ9iQ3OpLhbATK5Nt5p6YYrHWB4Ai'
        b'T2TRYXwvoCEGGxzrpWoe+L41WewLU7BbQHSnb1mgA4c5Gr/tBuA2PDOGj+X6/MexVa3hV/X8m+wKIvK7N2KjQzrq'
        b'EYw9CYhtr9W0aEHfC5hgwXRgc3dRSCpVOxuwro5k8Iey4C8cHdAG4Ta+TkDbFBmMBOsEwOFyAjlsmIMy65SWwJxm'
        b'zdgOoB5igSmSQX2lkXaRbsWEeJMezXvtBuH2uRg+zy+vrs/6Es7l+PS+3OyYCHh6g83bdQbry0xeDIg5QmiYDCdD'
        b'c6++e4HqGiAiQHjjtUcPcYsUFmux4/hNvyG6za8Rzq8+NRu4O3MCWTwstj2s+fn9+5X7ho1tWZ0lmalzZGStZAOU'
        b'uu1QiQKpWAKoCmSxywKYRHIGUcw+WTUZnLn3dHQ6cxvMO/s6Ae3s11ApIrtB8jaQQQfWLlApScrsrkNHgRAjAzeX'
        b'x+gKjYLV1kt2NnbQytEmuP98Qki7oH8u4pfLHz5/GB+vscT+Hvv//OLjYV6sBXC9qSHj0/dQ6kVjg/oFihTwYgBJ'
        b'CR6UC1xdMUEFaGOwHutM2GLC/+nbEHN0FBvgO/9yUexI4wFdb6DRmwvFAfT5726GCJZYjw5wD81hBr5rCT0ICGY0'
        b'zQwENaBa1IwOsnB0EBveO3nO6jrs/oJF9UO5HrekCzhpSnHdACAT0NtDaGTwwj7waSMCD8HboZ2im02BqNUUUJvu'
        b'oHAHjyPUHg0rbsN6py8x/O3Td9nGTGXbNQarwXSw2QHOmAbYrgOxna7w5C4LiCK3fkwuOFN9wVaYEpJ3ccjRqddt'
        b'QO/C8wPAn/uMvfzu4uIP96ZhOweK0ym4uBTsWMip7Kdg8YiWAjZswYHBtSBGIjh8yNNO6BNwGgUr7vKUKDZ0d/EF'
        b'o7iZjSyIY9aQgpv4ZG3xTqdijHWWgblwULD4lS+Ei5KUqspVMQkCN7aAHA0Sc3QcG667dEKubRd9nEEAjU/XBT/t'
        b'JjNlDdAiJuVkc3QZMD2hnaQDqIuMkor0pkhCkOEh9xhTNdlDGTaIePHiNajm4+PABvqnf/2Xv/3n/2g/fncvdvxu'
        b'9+P3bJH7oXWrVAGWQ5JryUhDcQ6QQxB5FairiYyK/V1GAoIYdSnMNGJuUtsMxXtjHjh93/EhnzANP5b3570wq55d'
        b'ff7woVzeEKsgJgqpFTmUdwbDlBS8wyJqSFTOmoRdU60tYFHD1Z55loKNk2VCiQ0N4CT9+OPejY74U+jI4RuCXX04'
        b'vz5Ekbma7AigealDl5cIrQtiBeHqFFAWZsOqMTE1ns0D0ZF+W1WsogBm5W3yRo8+fvcbA/H2xWL4Tfa23xaVd6dH'
        b'sSWp+6sKzByRUIb3VH0SQSJ1UXkS2qJH9lVwJ+wOzIjH6k8tQ2b13i32NyargHmUo7Ot3+8Q/OmRXH1Gorq6urg8'
        b'+1A+fTr/+MNtOJ4chNdOgPHcgoUQqaAbWFRB0pA2ZzcBmyTMgM2RVZoDB0zFm9RDQmiuHc1B/MZBvJwQD5ISJuPB'
        b'eKzlFU+vubiJ2QH7EOsxyAmMm0hRvbQUQ8wyZgZttzNAwU+PdRhdrwnzNgH3R8ezkRKvJ8TzCeqpXO75a5NVl+dX'
        b'tzQXCgIERWMc3TcKc5A/3iFie+cI3WeT+on94S3GbyogZgbIqNkAQjFCxbscjp+cjaD4U+4Sbo627uTk/cL4Npvx'
        b'2FSahl7qwGedJ48csIl8zhIKWGLV1EKSHLBjrKr2DNaLeQS/5MFq7/5o8eE3ruLjyRj/sW9M5epsXo7xX90gvYPW'
        b'47V0GNFkDEuzh0JvAUktYnFifdkE0AT6Y/JGaNNX5O3ugagjynTiQ+r9+Ky2MRafTs8F96KwlETIaA6SNYL9+oK0'
        b'1kF+IXfFGwm212iQgLuLETjTUhoOsAIB32TyZhRgaY+fk+0AwudTb3juhfJFr2MpMRakWm8pOzLou6kOtKqklL3x'
        b'PlqeT5SEWdDIvOxLUXU9l+gKiEsp4+hYNsSXZyH+Trk2Rnwnl7WLj/hyuz770crZ9edP78fVbV7D8DlPyduYAtQs'
        b'5G2LDYo2GGMhWjorIrBJegviezABCdpPANQYoGzgPz2FenRsmzj+Xp5+5XB18f4tUvLXxBh8C3PjfQOZGSACoGSj'
        b'xeGxnLDcPHARGqvW6e2McWKfgN7gN6pztuaea2jGytG7RTYOIE8/eLg7MQ07n3nsNgTJvKdWwDgw0rZWsXy8A3Dk'
        b'Ia4nCb6FjK1ukayr7UOG77FAj0H9gr+FlACjR6O/bOgv/vnr6zaAM0Dk7VLyAZAfhxkNY6wmgHZJD9VbwUecZ29Q'
        b'8t06DhU5jek4RttLYiJLDgkgQ88cHcReOSAvEcRX+IhFxOkwKhOUDJy94RsYZOOuAEfRXFgOlLFzKjQ8QuDvga/4'
        b'6bvpceI/9uhToC3m7+UhsP8oZ1sw4+q8U+AyHKzqcBYgj84iUs9ZLtOdhRJq6z5iBaWbUOXtxI+6fkcy8xYLb+In'
        b'fr4cN1rsZvlhU2PiNGFPmAABE63FfrfaDL4f5lIN8DF7a0eDBsOyHKFHNbWZNFgr0U3xR+uZba99L6fQAXwWn99f'
        b'n40/fj4HI+APXMi0MxZsOgtkspvZB14CuVZBlUE7JwS070wJnTerZTgouB6kROVNNraZ92OOdvRlsGx8QOIzctoD'
        b'4fwmNVKyEQE56ehiJ2U39yrkafj3xWW/TRHGOSQJC5YZsOxmUosU0ULhHba3OnvtHgLa9qxmgPUUCImRQD794C0T'
        b'pNso/fiZ2ViB5JOYGhYWiFp7f3HFrXOJnzv+dBOPYHFBFpSuExmsW55CglKKDaFXlyp2TkNaLGoa1CYWFs8BQSFq'
        b'G+omtqpDBjy6bGI7Tvtezatkiut355f97Ah+iu0krHURBw5koPMiNF0yMZehxieTE3hcoSZKGnwCfDmJ1ueWavGt'
        b'glAg28ej84VujEHtqwR9+9e26N/e+077B/LjxfW4PPvT+cd+8aezm8Vw7/PILuWINR2GC4BwzG3GPpzFWoFUwsfD'
        b'P+BC1xJ5tg4lgt9wwERww8m7jxJKtEczEN0YiLpf4/Pgtzy7+HyNuBd6Hz3XAOjIFDetuNSjlOYACCNIrGFYo5Ah'
        b'WA0girGCqpSAXeUg7EfCF0AnAR9Hx+z208j/5q/300j/YqeR7qYY2NvArRx4AJZrQoptzSLdZGeiJKri7MSYEKsI'
        b'UH+A1vuEpR3BnKNpo3jfp8wHjiM3xqVPv+h5//ljeXsx5/vzj2O/d9vYO/H8FupcdEAJcI9Z5+zbDTwQDaiQmmBN'
        b'KrBCNFRIFVf5oYO2kISBzvPWrdpQNbSjjyZ04116ynXPzoG/uZVMYCzU6Q4cGDBQKQlrn3Vgr0Dpgkz2CuqoGYkW'
        b'CjlU1qZ1gPxQFqgpxNcjRPiwlt2hsFlPuPYB8d1kyNnVp9Fuq7fERhacytC2HacqNlkyTWrVGXsMDnmhhQpiVSw0'
        b'b22abdJYmjfN8HLOpI3Z/uL49yxwwq3P+Imb+zANtxxeyTiwQduQHGeuOSJjTyDvCFj60zRkrpxK5LVu6iCLDnlf'
        b'sQTVW4CaH+ZR0f5l/PuOOOG+52YBXYIQ/HwDyx/6m+/AKUAxoEstSFALg+XYpcQxzFQAM6gEhi0ReQmbm/clHcNv'
        b'ZjYVrRUpezbj5jEx7FshnR7Dj+PyfJ63cucm2iVvwCyMHdobpqJFClc12bKczoGQWxZmFYtPvII0IURs/dJSMQG0'
        b'EfPiR07HRLGXLecToiifrkmNxk+jfd5IXx/t/OrOrXokQAaQWOelEQR8LoDJiK3tXWhgSbGD3dZQAU1ITCDfVcCU'
        b'QPhyalpZ9XjUjOxVy+bU5FSur8eHT9e3hz95u3gbIDssNAFXnRVpCOsGPG+oVmhB3rHlEpChvMlVAOxuSovISwM7'
        b'fuoxo990RLDPH/0Pn8tlZ03/PiM3ajZi/MPm0iIUBP54REblJY+LWDeju+wU2sdpk4hJgt4DaDPFIhhgYI1N3FE5'
        b'dRMQwb3c+PfaEvGsJZ1akrYiEngLCK2g3UbIiZEjZNHwlKb4/K2kOCN2tlarzpo5s8Evj8qpm1oI/tTV856lMruo'
        b'u90AfLOTJfQSQLi8zaNkZCbPwsTcowFUx9qwvd2wLJcBtTBZQxGARkuG2DbiMSFsCiHIS4TwKxRe3YGzDY+Dnjjw'
        b'3122H4WFicmayFNBTQloGxr4aAg2+ho6iRz0fs+gR2Cq0mSIaTkUx+tcHlu7ZPHHez0q5bD4mNz043+5c1N5MW7q'
        b'b7lpyA7RIA9GzcZuxNTPUVoedlJfVu6BQo7doJzdrFZam9VVntukIOB5SFsPcNMNxUI4mdDdyzmCHIKsY3VM/BeE'
        b'2XmQakAYFIJPzSNzpljyTANr3LcQoIzAppH3A9O+nQL0HscxoY1KhBOoxHtQiLMbln2viBJ/HLlHc0d66dqo21ut'
        b'WETVYAKQQXvwvNkLSE18KDWKqVBBvEQvkPwFayrJcRtgn4b0slH8qlt4IxIhn7qQ7k2ARmExVYUymDEVwR8odTbX'
        b'XchNLYiRA6xZw0ccvCAPBdPROoSO74izKyfjseeOd8a/kYdoTpEEfXwa+H9bTd7dZQTpCArhpA6HD5svnSDLbOmO'
        b'z+lKao73K634VrCtg8HMBJtDBFAXa1waKfvHTr3uBLE/dLIvGsSvuoo2EhHdSwbAf11cXv8GS2pjFNG/TDB3VcLZ'
        b'j+62CD8azE1ztnnn8Td7HTY5yPrBFzYxpWlb6hBziIE1SL2qdNLwQnGKtWZAo44iF3tldJQTc9Q31I4FfwbH8xAE'
        b'UiwvhcNwHWGMgvwJPV0TAIQPg0z0JISR9BXghgnCBsEvi+ZxlO7ciEY8gWhAao7Lq3F7LnbvhNv5wKviJD3GyLfY'
        b'0+NXLZvmeOpVWwPPM8A58aCADZsHNDUA7lQy8gFlKLTpUbptr4GOpwB3+XD+/udDKfeuoLmuKKKDhyJ8wzrnHAB8'
        b'NVdfLbjFwPwkUBCLPY18lqsDE0FwoB+u8Nlg8sUHB8kR8nBHhbHBdzwBvvfx//HzxXU5u/pYPl29u7jd65aJt4Pf'
        b'2RSr36rZirogbpoSMuSZhqodihPKKADTO6vgoCr86BmkqbRaUjkqjA2/YzplNh46bA3qsaZsDBZbI2GdYKOExqfJ'
        b'w5gMcAihRgXZawO7mQ+7oFXdHBaj56vgGXgY+3AU+wd4Ncr7/VgsPvPC5P3bL9/p7kU9F5TbHqHFOKemEBw+cOBE'
        b'n947kbmRpox9XC2r3iIzcWthCpYRQrXNlJGw/+MxMdz5yXs46YRKipvN/s3jSmcSj5p8Qaadak3Bkgp92KpGKdka'
        b'SWzxLnSQxdlzZ73irFIj66dtGW70GR45Lrv56eXzT+fvz8vlzzciO9nT89dXB2dY76z4zrEIpsNHl3WCo7fhe/Ug'
        b'ts5oTpDboybhe//pWYLoZ9SpbfIQ3GRvnhbNBpDJnR4NC6D17Pzj9cXZDY28K8AleWBk84D54bE1AuBfB4hL4/k5'
        b'3Rg8Nv9gMUm2MmuyOWtxkVeTBuQGSdylp4W2gWXyLx/azRvb794ESVVcbSmVkVkU6gxlXxEaF0hHouhQWtYY3tKH'
        b'xD2ElCGcwB7cY84G3wpo1+hJXmWy7hA0CY6lY8MMWyZfHtiWk4HEjYJMNmONraYIBQ8qYDqyhrXg0M4BVZ3PvcdQ'
        b'ZgpPnK39DXF6AV6wR7QFt8e5rkSWKrJAcbhUTVXWVYBJe6NQCUjmKYLgTPwMZJdp+R5kIMELKCf2IQVEeqx4/Jux'
        b'bRCbwsvH9pstxQ1tU3yV2VqWIqar7UXkPfFpMaQ+EnvLFQJIoYRGNcq4sfZsMqpFQY00Gd+C6vSkqE8MbhPSKZ0e'
        b'HAub3o/rIfHhzKgOuDzA8WyMEA4WU8anS1RyvBJW5MU5RihVRw18N+a4ckGlpIbhxWAewxOT/q60U36lAH+zRblp'
        b'72xeb97urkxNJIWgGTIiiEV2mvLEvMVoqPds69H44lyJ0FRNhm4cEZTKRV8IbcN+fSb4SxFu5CO/APnYXpJKeGRZ'
        b'aqDzAt8MmO0AN2qYqbUkLCokRdc2ABCSRqMrjHGAdcvL7tBmD6zEjf6J0e22JO41ovvN1uTGQrJ/pRn7ekFmaK2S'
        b'fSzdIiHm6UdIzdvoJWqC6kLuTMorzcwyopKRfzptctR21t6Np4W3q/n8ApRkK3j5xQVZHX4nYsgsKh7TWCzCBGIP'
        b'Kh1Agh1yZCm9F0MwD5g7ulJE0OI4fIRwe2J0GyfJ+hrR/VYLcpf7ObzSjH29ILEeMUVKJbY9xomTj/KwLDtL9/EV'
        b'U4uOyPMM5MXQWKgWITpBtBo+iKdO2UZM8rOIyd/9h//B3/3H//l/9h/9p//FJscsBkTPE82uKTYVpJfN0QJ4s3Uh'
        b'ics8Xc0uGMOawmhM7wGJrwOkC43MIKnzY9RjH8f46XpcfryR/vlZrOPw9nbX4JsPEA3mWK509a7cGkxF6H8DRe8y'
        b'tL6pfAgmVUqN1pnRWEBWPORkblbEqal5pthIOMZsPSg0djo+mn3r5JeP5rBvoPtpVRQtUlf2NJryhKY5Y+Or7oYN'
        b'E1vpDDNjqdkB4R9A9Bt0igidX54QzG4MZMxLRXOzQxLLMuqcWGAYbp4hAGgbn0cM7P7RzYTcYoWAmzpnjqlgJ80C'
        b'dtsKcvds8bE6zjWK3RjI2BeLYpsK9bzoTRK0DgjDrLZspQEhqwmj08ugNR6/iCr+rMHeNxE0KIwMBttLn2D0xwex'
        b'WwOZZ5GE9m60P3y6OGeF9L3na8nR3qhJYQm+502pLVl6tWYUAdDwAWjkW9Vu8kDmNaMK2FsMfL2Tsc6cH20cH8Tu'
        b'DWSexQru1HbfWEjO85++WGEGwmTCZ+u7xN5rTzNANQ3rHeRDrHXz+qqsP1RoevFYUj1XZCwkhgRmN9ITItlNgYy8'
        b'aCR72Qn9czokdwsmbKieyVdi4t4dxdrKuj07oOdpcZbElFIqH+8Eo71ox3yZ4wPZ/YCMvkwgt65ZmbfXEAmtDWzo'
        b'BAREAka+zcWE2AF0zWbsFT4xwM6JZgZQ6NFibtg8BuBjn7DBdxMgE14ohm0a0vaOOEGwaBoGAwQkb4c+WPY+IDAD'
        b'WZNHMzFNv11tBbocQcKaXqTWoN3I8SEcjP2ed7Dw5R7ihjQaPonyqWFXYIdOA6GCEeUAoAPbsBWaDOBNf9jREm2I'
        b'U8A6w8qxtDgYgt0wnjD6g6GfSc8fPk/QbxE7cSvbEHiBlRKZUo5ITJBZ2n2mlnR0LQH5GLU5bHkF6AUu/O6L0KEw'
        b'PGHwu4ufyScOfvcGEKDCROIP1MNa8H9eMESHfVptb0aUCJ07xWMLUSLf0gLYfPat48806+MTxr4jtH0WQm9Vbwdn'
        b'g+2dybj4fCs7oGuBDEBh8D2LtaEBEwC0hlwyNvJ8UCJNhr01NbU5aQXZk/N0gaXdMyhJeQIJPLj3WXtqILcHaDTg'
        b'lcYXfgAvmzBS11pIHusekD1d8pBS1ICFDwY8chSyKA0hCX5B8kjzCcPf4dm6k4f/KzlMfBXADs32WdD8YVxjN/IW'
        b'9Lr88MW7YKvptgkCD0BpoO4wKG3Rpc2RDwoCqbUqNgDAeICN5+4MfsuL1jAoOtS7pyShHZPtszD508/X7wgBYNu3'
        b'Wcizdpj6rZVIywgs9xg6wEAtn4mbPPPwEEH47K3vWlszjk82UpfpTbOluSeMfgdiq6eO/uAKTIfE2HkslRIyjw3T'
        b'Ntb4I8tb15Ehp+Cj18YnpClNEA2+ex9gFqH7YPEbT1n+OwLbZyHwfoFeP/cfxs3udRErZ9RhBu2dxrR52Baa8nU1'
        b'XYD7iNjSGQkHIIa9PQTLX7SraKP95pRg2hOGv6OvfRb6br/BlX97ABBZuwecihAyzhREYKUNiwXuWQIQQ1RHAz7B'
        b'n+Bi77m0is8b+mda/BZoRn/C6N3BTfd5DyjBf0YrX+pIEmuUeLvfYoA6MB1aJUx8wNqj480DOCcPNHz12M2YHHzS'
        b'ZTTEQUc9TAkw2z9h8Dv42hOeSh7E2Y11wk19klNHFuqjx8ct2bBcJzfvDRI96BGNaPGp22pLhMIU/OkZSi08HYBo'
        b'iPRCM+B0Twhlx+JTDBzvhXJb2aPB8G1PAJQlCyIRkIh88dFCfbbsSonbi2hN0PbYDKXlPsVpjfhXAJT1CjR8Qhw7'
        b'FD/Pt3F/JrZ4/J+1P32RaRs/8ol33DoGyEPibCA5jelt5tvOCtpRgMgNvAMUNUWXc81EiGiFD3SfEMuOy869bCwH'
        b'70mKnOka34ZRnrH4bdYeNQYssoRPHhKfXBvYAMGpMoS9GWpTrK6Jn9DaExB699uzzp8cyi08A+AEDKL1zmfOknz3'
        b'LXEiaC/WsozkoKfrtDHjC3nityKPbSbUAzIVixWeMP4dnp9n2Hh//IePP7FqpLoB5u8rJCVIBI2aLV/zsBJ0+hxS'
        b'HXlrWYLkhb8RtmttoJyEDpDp/Qnj3wH6eY6Nf3p3fj2uPpVb9xDkHxbwRJ9ZElZJH7JC4EAJaDU+D7PVu5TkJh+6'
        b'jQbC5LFvAOXYN75jr5cnALTbAfp5bo13Bv/6FblfjXzH5uc5NP7p3QWWDVnpOOuXP2+PXW8BYjvAw0eONRMzJJql'
        b'ZSF4G9QA+GcGK2LFVAAHackNCEvMASR0kmiRYVuO1T1l/exedPZ5Fo0PBLK7rPPFSQTFc67TvB8SYGYEZDByV/kE'
        b'qPVIh9gqIHpFnRt2ADmULv55ZNoHPIGo+oPpfX6xOPzthCS+nwTHnnEW42oGp4uueASgzeQ6scsBeOJp1QCtPEy3'
        b'dffTwKQINJM+ZUIOtvfmBQPZM5MmEMDkAAdQY1DMrYRIC4aJgCqQu/FVtPOg3IE2+L1ZUBRyQL7IoplQHfEJW2R3'
        b'obPPszn8ZiByd0YglNXLzHamjKzpQfeqbXxbH1PwQntizwP5ArBIDooVGwmZpiFww4vJJwSyo/XznA4fCORwpsp0'
        b'mxxP5rILkDsTvNyN2sb2OjFWJDRXtMQGaeG3snu1ZOcdfyry/Wv3T2C1u/WcfZ7R4TcD0btJq9BFhl3Aeg4DWwLf'
        b'wUUA9eZWDAqFSSnMz9EPyFZePdaWMU+zgUyFp2Tf3XLOPs/h8IFA9tsTYzfzrxmwd1lzbxxEHhBiBj+ScQmfeQi9'
        b'hWHMRG4GEmYkOm29zLYX5uYncPPdbM56faE4bmfD0oAjba9wR+GbM6Hbl4tYMArc1mIreB9ysANHb3VC3o0BlQH+'
        b'R5+BXB69m1+j2FHch5eKYj/pZtGjLyBKHTKidFerxlhsrzbXYsFUJh3BeqKxuhsziKY+XTI6SmFpuNGnnHTvLnPW'
        b'nwzony7HfH/+w7vrL7sjQkzwZtGZ4FxuxreJATulcxmUt7EqBZLDFBt5W6L0SajgNTzRzHMCQMqMT5AXu6ucfa6J'
        b'4fu39aL/fHb+kVYO2xXER/zDUM4wMzeu97t31sGaiX1hIIfwrdhFhcXqgEoIXSho+syF2MFZoECqN3TkiDRhoddm'
        b'Ud7rxT6wex6O7+6rn/v389bnl4pxf7fCt/oX2wX311EmmlmMsjHGWgCKAg5Z+IZoKigaeyhhYXosUN7nsQ8LfpEi'
        b'eynOxNcvIT/mC/bNKHdWI+a5UW53rGc/Wn92NT6Uj9fn7VuROWouEnvOXeULWZ6aYyNZw36PYVgeboWMbE6HeZ/4'
        b'OhXLFj9O+swFcs3oEyPbaY7Y50Z2z7DjUL2/RXtrm6+WT3YMmb1TaFzoABDnXIyWQTJX8qQpXzfWNd5dJv5eBTfI'
        b'DnGxo9pjJ6XfDOrQ7Mc9P6jzq+vzjz/c1lkgm9wNSiF5v3vTIqSB46u7CQXDy43GNlFMliOIY+2fmSWGIKloZCHT'
        b'GLxLg6QO0h8jo98Maqc/4k97BbO95zksR3fTXBEbiq+NhgUpmcl57K6AGUHmQGInIwJUJSDANKbTTrDQ6zUYj99n'
        b'xxaQifHEYHYKJPJywRzotRKvePrIlnezsGNcbx1Ck5kuKvsuWRPAD9gHZLJCExK1FGux0RQUqbT5WJeJbwaz0yDR'
        b'lwrmxmGCJST8sEsGOvFKCojaPCt2At+BQGwOwW9VwCVIUHBpAtV6pC2vyWJBiJ66xnYi9FwbxHuRXLXL80/XZz+M'
        b'j+Ng+PyNjBcpISKYQnSzepeAQGDjldV+rFkoQh4uGTAViwLCktris0NoE6tUalEQ2yfGuNMkiafFiIywPev6Ub42'
        b'fw2sYPJWB2iq5jp0u8bikUBOWhrSgczq2oiTZ1NekQG9bNaC+GkO2OVzKE+MaudNkv7VeyP3bcDdeYW8xJu/b7zs'
        b'3brJzkQjysAc7jAhYNze051bgavYagV6AmmO9/K84YAqx+KbnvdINIiMT015h0oCNaeFdP2ni7NP5frdWfmAn7B1'
        b'2vlqAUa6XAyTtFc20BkJiMTlFW3DjCnNmEq0oPGguMPnzFqK3kbLyrYwsxbMXXtqeDuT0GcziW+8kN8pxV7Pev7x'
        b'0+frb77S3nqKedcir4s7z51rmA6rtWuq9OIBezIlbp1nZ8cCHiVtBkiDDT6sHc7IU5PIoSJBn00xbj0ADm8rNhDb'
        b'MuQNKPNBVphVKzS6qbTRCxYSHvsLvElpUzwKHzk7583IbPAaC5ihQUpxEP34e0/l8jcdBv2LBnVDM/Z5SuDo0ntq'
        b'UZQWB5gpUPNeoXg3Hk9n+WAiSH7Edpz47q75woqkYWlg9dSQdqKhzyYah6u3L1vs7Lbo8/q29Qp9men3p6NKbq7S'
        b'htiGmbId4nVsXSALkr+tozibGhIoEj00v9fJ5s9mPnn97ZxD9cS49pyyZHzrHM0bKm9BQCqQ2C0Lt4diwNIBW+qi'
        b'AMZcw+4C5oBYFQQy4vasDGoSH4LPT075O/nQ8DIh4f/9fHX+xVF+M8sHewgqhu+bWY/rBJtpeyvhtCdeWIMTzuHB'
        b'q0oPLQ2W1bBtejLgxKE+efXtXEOfzTWoGLGjLj5/I8mzFAYpgg7RoOnN8RmVNUYgMUbzILUO8hCSV+NIpSafoRzp'
        b'K9Win62PzYdQnioXD8UQmk6JCGmC7Onj9c4LLz+/5838N0z0zfYKMIxSJ5Uu5L7p6rZXLpXEqZok03WszEpXu8EO'
        b'NT1sNzAKqc9rKP9U7XgomNB8SoS3Kv++MXNg2Whju5nNR0fLrJEJDzMlLKVQNb4CkBPbmlT2ADFBbI74K6DHw3v8'
        b'nCcL/J12BPP8eORsV/l9vL8u32gC5tnnBCwiYtcLe0TLnFDDiAXZYYAAB9Ppe9fpLp0asgjgORl10JuzhZJLeqw9'
        b'4bfD2ulGsKeEdfc7H7zatyOMs4a/+aXdMqsS6BcN3AWnoCf79CwVzIHmHUh/rIvnGUzn1RqbbHmex8QKmUYTRgui'
        b'/9Twdn4R3Cnh3SbEn653k/Br/IzbHff1PG5V8mCKECpRALozegtSVR2hCyiGfQg4UD4YiZTN6moWY1OeWMHYiiGU'
        b'8FTaeKjBCM/mHHcPfe/+hE/vCijI7eFvIrC8mW677axQWGDC0JpRtqol30ztrGYoxkqnIfhs1rta8xgT9LCDV1nz'
        b'ZDl9qMoI8tLBfQnL8Mw3xlYMhlppWypAKwCzL5BlUxK/4nyajiXF3bXUje/GZM/XZ6G5Hop/alg7+XieL+J9B5N+'
        b'Web14WTg3Xj/iUe+9w57eaM1OzA5YzNgMliw2z3pBY2Y5ZAhgWVS8IXZIztHOWMjFvN0UtmU6DF/lp9GPyufr99d'
        b'sCf2eqz9PP/BoyO8NS3/7o3jYS+WJkhuH3Uq1XPpMTW1tNCqrWrPc05XC8BwCigZCEnDP2y7YOMzQjzgQjw9xIpV'
        b'wtPSh6eRhTFvXOESLDKAz1Clhm+FbGaDMhYo29SDYROMIvgfHZuNlgZ0PBpQcik9ds7zYIwHkEivHOPNfR+9JdPW'
        b'ymfWYbQC2hKSi0B/ClvmhCjJsB8sCDT99EBg+qQVGiCfnTFqy88I8gAV+QX8hHa3zH4Q24eYb1JN2BpMus72SzNS'
        b'lQ1FBh3ZR1+QPjG5UJ0luu47SJvZmgUF6GvRwFckpbhnTeKOEPEF/JJufCj7uDz/cSefPGK4/ZlfeswCK8oMUGid'
        b'b/VYggsBPvoYvI0umtlwpmew6uQRXrLeYvOyXUvBRsUUh8fqEB+MdIeL5/kk7pGSfO7f/obP3Ot6co933/hhfyl6'
        b'8pFMrgvbCoBhhxxDKpHu3bw4m8OlzhMIJKJo8b+iq7wSKCDaLYEGOMiM/Ng7uQcD3wHlef6KN4E/wndW5+/NXYSd'
        b'EqER+BzZ19lzw38Tq8NLm2AA+DnKIubtRaDtJELUHilQ1fPu+xlh7go3ntDj6s6x5tcHgbetUNi1k8ZEoxWIXJBk'
        b'vtAOGfII5GerUgvBuM3jRwNIzwhatoaw9AUYk30LnpOIDt15/5N/d/cc1hfzHJazH+NuOxwYCZsZpORoUGStNXU0'
        b'KZ6NCNNkwXNIEakIP2sGGbHUOp11ddKPCXNb2BM9PeA6fEg1G1/708XlH7a6xqu3l59BwX4qZ+PHD9uv37ZLdqC5'
        b'/fqhE+Tbq8v2tl18bFx5v8dvXY+3fzq//jiurn6/eVdfXvHuMJKJbrWcyreorRWWbSY+ooKIyOCa3bD5dva8fgMI'
        b'Zh7/xVkym6NgNkN7bPW1d+XjD+OmaQ4vKrD8t457Kdwcsuxz87IRVszo+z1GaxOfDrM+KhbbYhnYPLNXJExgP+SQ'
        b'AXLkMAH+HbJCbeWxjPdsI+0bC38KO8I/N8b91GX3oHxmjO/P6xaJF06WiZ1l9U4SUlzeml7lUKDYq2kJitamURO9'
        b'uRW/XR27xItEVt5jrbXanhnIoS53d6F8mcn6cNG3uFwg68TYI1Qe9FuSRMNrJPxhfWqj8yabBj6+Qy8YSF4+zMp0'
        b'IM8e5GWAr/lnx7UfIu22lC+/zX6PvwPxvi1EE5PZWnu5Cd1gO1YiciAveiGFPGhYDgAxGexWOh2mD+mxG0zsVpHg'
        b'eYNX52NNWB8NVA64lh8JdPwEJnnO9H31Fv/793OUq/N6/h6Z9e2Nb+jv//i5vD+f5+Nyix0J9PdX7wp+1O/3FlB7'
        b'rMIEj8XatCOnjsTmCQ75A8DtvWD+MDCx1hVPZlOLd5hgiMbexUPd99Bitv65mSXs5GW3tXyZWNn79OptJVQgVET9'
        b'u08/8xRNPQtlqql5OzlqDnvRQa6rq4qMk5BliCsh8tIYnwTYSzVZm+fta2NJ6nCP9bPZeq7eic4d5ODub/mSsXGl'
        b'/v6rAC37U3/3BiDmfZ6R72oRSoLAbZqylFG6N5NNAyZo2bBGqlbPJ6zAtoAdCik1H+sT8HWAuxjcLS9fdEfu2cY5'
        b'ltgO9pOoGiZfgmX2s5n0dZMxLf4rU0E2ARCDk6cplEQnLaDFAACCkshz8WDfhQ+aXh6ak+07am9Npr6dVVvMGRvo'
        b'nhXspDPb6bqdxqw53NAVj+/yw+cP+43l3x9YtjWHyntLDTETZIK3bFyENNr5RJf1rGAolg00APg0xFHrB4AF4F9r'
        b'p7c6oD9a98B9w4eLHzeN9g4L7hqJzPubTPOgDeZLx2h/ixh37pL0V4rR/RYx7pD/y66Y/ptvZQ/Z5Qzc+d5zKIE6'
        b'4wZka2FbgxIhTPDdJlp35rk9d2oxpqgAAZCXHkk9kW3G5sfeARnpofZTB7K+y4J7RT5XN4kzvlg4NxYihoXuigFL'
        b'mb24rL1EYxJNmSa5SUXSCAGjT17NHMZVydsJhYekBdeMEp8ezSFLpudEMy8uP7CD0yUm/MeLdrck6uLz9afP17+7'
        b'/umaRcqgKwNCTSMfUJngmmIGNKWqCjwrIJadvkdmJIv4WI9XK7gz2WeAZAXDCU+PbD8o+2XzyydGds+sTnc7odGc'
        b'E+T+YkY0Yln5xNdIWpUnoCbNbhHa8LbQZz84Z7Vgf7EkLD/0ePix0PbjsV82wPQPFTUA684evS43fEL6BkDtwZ+r'
        b'B8WAeJkDGhuoNkI3k41vI/KFKJMP1qwhZfHd58EjFYDeQ52Z70V2GMV+qXizuX7Z+PKhyG7k9eE087EYYwqbr2fv'
        b'E1TSjingydlqE+hZzzaTjZ1EPZhIDJCsYJ1IlSAjI9iGjAjGUh/yFH8sxEMbo3//H+9HCo+mxbvNRSWa22Djnaba'
        b'NyHrfpwAKb4F5k3KFvzfz6QVYhtxmAKlMHocVN2YOTZIRMaxle3nle+BIF6r8d3pV3Z8N8cJOyX+Ze/Ob03Q/qm8'
        b'vX3HcOuOFtx2ukN1ZhTMnteMydGnp5PNV+yv7sFw/YyhYB1qruJYmJfAsJovtJ956HSHU4CpYLnzXtHtbxaZf94i'
        b'2/r2fknk5T14Z2l/AGf80oWPp+a+x66IJsbNsSTj71c2vTapGFUkOqwfdo6a0abJrI8Mga+zKUqbD3YUPIRzsDVT'
        b'uQlGXjyYvZrVqLBbK8blQT1A19lytvLCBuoK0bHbrYZBS23LTtNpRNfiKHTaRlZHehht9mOD2dEo64sEszxkv0Va'
        b'0C1eT8UpdUtlxQdBaMNn6EpTW2Y/9mTIera7fGSEbpw43uEEPnbC7z1Yv/9VSDsM5fBKId2Uuye2prUIZzgW28Zs'
        b'XGfDgJk9gjOSm1ULOWkHmCJ0pkFyCEzgCM+xo8DDFhxfhXSAn/giIW2/uv75q1miDQQIEXYJ8EVqiFboPByCtxh7'
        b'LsAi39lRLhRe0oRcijdx0PJlRsgyIzHlcmxI+ylpTq8U0v6WiRefNOgwziKJtQxOFw6W88hokUYv0YMGAY3M8PSE'
        b'kCAdf3SCqkZvoYansceGdMjU+YSQNlE9z+f1u/tIevteNvNYXgV8m8+VTDA90h6rgOQlVzrvF/jwjIaRLXdvhS9F'
        b'IDNAYk2O/HH1wVOLrwLa9KAz5nUC2p8rsfRA+UCzxmZd2can3QE5u0AtuEpnqmkcH5tNZAMap7aYGibOsepzjgfL'
        b'fr6KJ+7x2BPiORxWfAVAjo1Um0yHtK3sgAT+YiLEA88nZtOahpmpWaSCbnovioStCWuu+JR95GmGHhtG2sNwLxjG'
        b'LvDiVt/oDMtbJi/kDFsLl1md6/yKUYcPHzSGPXjZ5iVGIA+SNIt6kNx6QNY7GnryHoZ/gTDq5Xnnv/jS6l5F97Bu'
        b'QKW1CmUsgBfwrw6EjDYDfPA7KbUiFUKPDqRQzCGwrZDNPMltfoyjYdTswchLB3NQ3o6tNUofhu89QA1q3172GZ9B'
        b'x0yqTgJ796YWILohXdktkxd37MIFfpNCrv14TmD3YPTlgtlP028qGDkz2ACxlmzmRI6ulv1i6ZkShxaD1CZp2Diw'
        b'HmWYPvJWuRnENVesMzPno9nA/qTAmfDSweznOiT+rZiIvYwPe0KrWBmYBprddusnvhFUgPpRTUqUL5uwcbNP2nEb'
        b'TchzR4fi91BOoQFQMRcfzxsfH5U71Qo2sVrBK+kjZLKvcQ5ACEBjtiZFe6JXBnaEH7ONwdasofUGpUOfDNlaqIej'
        b'gXJ/I+BMeslAdl5mssPiAq8fQEfs/aYB/weuNdTH4iKgPlakhsJ248bj48/R9Fy3xoe2+wESY46WAvvLAGdOgfyP'
        b'5ZpdldkYA7/aD/u/5maG76danR7zAE6ZRs8Be6En2ye+0Otk43rbMlQD636N8PKmI/FBZU7iEgDo6LB24Lfm9cJ6'
        b'ZZufr0Pasd+egv37mVBfYrrXDtXxbS2UaGwAEd+csr1H4KEaMJ7NSRpSGI3fcgsaRXso4NO2FbofQH5jeUKkHh3U'
        b'zgSse62gfvVZ2jmB9acvvMMBHr3tFwptjdIcHhtFPNQ1slj2w2EKSqOtBkQ1Ejk2kRZsrToaXzg05I8aqmOv+cFH'
        b'o0czg/0pgLPyOiEdjMp2M91sLQgaW7nGrhNIVKcxPMcuUNLSMPAyRjPgQSNrwEADlqmmWLH6Zvf56Jh2gmBPIQhf'
        b'X5bcKJytAQHSm4ga6QWTE0uNUiJUjatIcuxjCRZXorO9bRVjOk0zAh5kLE0Lx9GQur8AcDa8bCT7rPCEoBnq/2yB'
        b'RGOGWukvDbJWXY4WXBMLDDrbq5+KQBSRWT58MgbqISCdx6MD2bmBjSccE86C5fVDub4tKqaDzgCpweYv2APN5cpO'
        b'D8hUsQuGK3RM9XT0w5/odGph6YqX7Cso0KytpaM5wV7D72x6kQAOHsdQF5wDVnD7zdfeIyOL5hLE+zl4puZBxapt'
        b'Y3MGsjOP5sCuZ/T4Iz3SZvdo2Nwr9Z09hQ0cHga+u7j4w92pSJYHnFIgIEOXCBgZfvMvA3MBB2vGbw9/sNXZm813'
        b'Oi70Foqn11enuU6pR582sWiKB+a//0f7gXl8qQPzsB+YY2unuPXAiol9DwOmwrNtpa2sc05s7Ty8soywJod/tlJh'
        b'4ihwSCvbdzZxZTxwYL6TF2dechZu1hNvId+kgmUSIig8KCQ4vAngyWIgs8roEQOOObJkpg2sNRBo8IBmAZ1bDbef'
        b'x+/pnbO4UzgLHXC+8UpHAp1rC9KMBWNOjpxyusqHVny9TWupnOn6hfxl80QuxrLKNPizTrPBPECPHR/IzlPcKTzl'
        b'x/Kei4nXgoeq5JssFcn4ARLYHZCQdjIgQ0e5bPhMRdkYAv+HuQLQB0B99sWzocqk1yubcJp59LHlXnbm3EmnFvtB'
        b'2D03+Wj3jgQpEAKgpqzoTHamkbplU5SQO9h+xIzlbWtjoTlV6LCEeKPHdJmjNfFuFOmcvFgQv8n+2H0indPTw9h2'
        b'+v115Q3bYxpvkG17Bq3C6BtIbqV3CLucd3bomx2sHdoqKOCDrRewNdizr00+YTyaye9Okc6F00O5+tz2qqWzD+XT'
        b'py9PL9hZiy5K4LkYGQccsJr6lMJDCwhMDBo8hQd8kfYUxYxcuh2GvVzpuNSgY8rxVxY7IXGnHFYALfj64KGIMjsf'
        b's4pBgNlsQ1ddm7SrcDP1YJEBbDSeQB/mUPDclEF1vZMZ+6ChDz05jo5n5yfulDOLT5BV5WBks+mty/OrW94rWGrs'
        b'qMI+MCnmUEtyLTkzQ6+jj2oi5kI0QaMgfWG7j6GWNTeDju4D5FhKP5qr7P6RzuUXOBa7k5rvN3+i2bDd3mxNVQ2T'
        b'fTrLCIUGPWVEgD3LaEzubA4srUJoVRq/u4BPwNaQZ2H+OzqmHfm9ORkvP/YN9q/u+6SzxwTN1WyCVsq0kwQXy2kI'
        b'ckCGsJJBw20kC2GH4x6wq1JS1kuBr6QAcpxqbUcz4t1R0nl7ek64H0bICrCZfGBWRrKRyhAfeGi1DlB79saw2Cgu'
        b'5mFi8K4qvfkdouQxpxTNtj0hS+/g792pt0j3Yvki5z3bCUSktaGdNqWJ7axEiZvBsI65dJl9YL8YvvGwmDO2F/Ns'
        b'3slbNBCf42+QdmtJ95jtKv/83uSWG+KWFs9i41lU8We5VXuGlORnyPSKDHc6PQxg0PhCoQ9VLV+eyfyYbr0oYqB1'
        b'Q558WuaEnl25s4YL3C0Fx/ogKxm/GNEVPryCFjNmODfcoDKI3hQJx1/X7pTBP/sV8kGifREFX5PSyPreCSHvcslp'
        b'c1lDDvD0n4USgD7j4SaUviS6N2Sb6YUAURB42o79xXeQR99tyk4e/OkPjG5DujkDzJDy7M5ZBfJ+IqXZSTskIBJL'
        b'q6F47ByAWi1KY7ZsDJhqozuoQ0IfETkRAHU0MMlOHXx4fiBfJgXoentN47aCNFYuqMOCaRIscpjBulVQiQbms/ms'
        b'hZ6a4RtGpWlcY+IIFQpUlZcfR0ex0wUfXyKKr7GVOaLy0j8DZaftACKn0MxI0bMb2xSpYNDwY7jJnh0hY2W0wpoH'
        b'+oOzGeHRXE52pvCgZeuhxPiecsbHF85Y2XNGxnKWy3RnofCplkc+LukmVnk7kQWv35EJvRW9KVC8Udg3KzCQuyJZ'
        b'V8jg0J2N1rbeLctswJJs58Faz3y6B3rOy90BPcG+ESYNdmNgic7xU7dTCX8KldhrPM/GHz+fg03wJy5s3G5PcNiB'
        b'0FQ3kposjl4FtmgMrhc3+M7NSCmF72dl8gsW2SJi0SY/6bMhR5PXvRLeiXnRkH6TAq+94N3JSYcI+3e7/wyYFru3'
        b'tx4+bLduyfHcbEocLLiBhvWlOWFTN2F/q605qbcFiWMi2hZ4Gjq6YomDAB4f0s4oxJ3E83azWTal29/53vHYyVtH'
        b'32Kx1MDBa9Iggr9cSwY1dwmZGjH5rRFgL9XMPkPC+qpsVjyRMDKdxo/fPzunEP8q+WLrJX12BLv17FrOjnZskGMU'
        b'6VzAz00ER1EDWZITqGhBnuz4PHzqNLOJlj2lK71ORsegXDy+WMrsh4z/7b+9HzKmlzpkPDzyVSQk1koFB55XbZXa'
        b'y4But+AwXQZEbYHUStp5spjySIFTXVIFu3B0p2wIdDz4yPewq+RV5uz2r22Td/8j2OfzxwuWyu8P926Nk+9NZ8Y6'
        b'jbSOHy5AegHUwDgULBQfoHJ2+Qdc6Ky41q1yHr+x8UgMzLrRCjiiPfrmTnc6JfprfCCbN+zF52sEvmib6LmGm2KH'
        b'urmJAR4LOuTQESQy0Rr2YsRqFhlsMGFLYJZNpg/k2do7Pqij1YDu1Otxq9vPH7/KQvr2/eeP5e2Nk8R2o7f3HwKs'
        b'f/Eti+xSKn7StrHTa727xDOO1NjPQIuJoiyXK65y1BmADqndgrSpkU2KNLSjSxl0J2AST6szvznfuInC4ZMmMuRp'
        b'axUxrho/PVLLZBOoItMEpWtNj3PvAe+H65avomiK4XoOPvZxfKX5YRmecosECrybNNJP/lZk7m+dwRBtNEgMkfd3'
        b'bAOrE4JTQgWlrJZulHRFYBG9wXIeW2/fjkkB+VLfjo/jsLLyCXHcyJNvZgcT+CQtQgxjghooYPIOPDE4Q1sOkTKT'
        b'jCm0sgZDlDpB+Gsr+Id91dJg2oi/oFEOO9QdSuadmn8VZ+VLFPvaUntCFPec7q9u/bdYeJ1papdnr4WFx3VCQKS0'
        b'daY0SIjB8QErYmqd5fEZwwdL975UPicEAJdjo9hXlp7ynuS+U/+BOvGFT+YBwZvcWcvXUw10UKej3SzF8O3FZFdE'
        b'640gPgMChb1tp+tNM5dfH6aVdvx87BlL/QtE8g1LYFBmllrz1dxgA+UOie3A9CTbiLmpbYYJeeFAbEuwebNmaiYa'
        b'cAu+/RFpWUs6NpZdM6qcEstuvHQ2fhrt8zctaxI7/lp6YA4eSeRUc87RNz4E95MnyyMWkDiIJs3eN5cQmfFt1MmH'
        b'kT3SyPrYiHZVqHp6BuO7wA+fbh8v8fAyW6g8tsCe0VSvFmrDsqia6Ef3M+wIqFsDoQuGSkdx03zMfOKeYijeHxvE'
        b'rgM1nBDE1nuZL0j26bn1u7S7SxLGndnScYZgsUtqCJMOUOBeQHAMt4KygoZ71vR4QyOCYdhRFOQpH73nd7aq8QXD'
        b'OLQX2FaUmu1dyIzF8e7I9NIgi2qiq1MU9qVl31mACfJaAKioSSH6aVsP3fwS3foSxS77NJ2+oujwcf9dKYgHzWaC'
        b'2dqZ9uIgfKDOHXa9TbYJ8MLxfLWyqJLFe7wU3MwHskK/YlKylKOX1a74NL9MJL9Smd4dIDxot3/5D3ftll9Ku6VD'
        b'gQgSFRtXtEBXuAAhnmlHDiZPrdmbVBdKG6zeH4pdkafpvvOsO0CcSmaPzi5ZHtBu+zoK5uRP/3eX7Ud9811gCXRm'
        b'LxfIh6i++DK9AQXxfmvyFqI2z/N/D4oCkmLUQN4I+LACNCyv+9RBsxz98e9EJNgXIIf3kpJAMvNoCp8koJk3XAo6'
        b'XmxlFSsTUcpsaw8SIkZsnmAdYSTJyESp8B6s+4KdfXQcOxUJp1CR93TavRFS90pxLfTtd294tA4YBC9R3hfwLNvw'
        b'mALS006dAbloFj+aVm+B3rMkzBz2PMvHoR/d0bHsZCT4F47lV9/aOxEJcvraujcbKtF7XgUbrBB88NWCDboRIrZ4'
        b'ESTQop2FLyAfmAvHN1GjAwz9bAnsHbQ3jVmOFhyHwvygJymOtZfEF6xglQs1xwSXMim5Ngofp8TUQS7YqJJ3PyUB'
        b'vZ0yFyCJhQy+ux0j0gr4l66B74Syc5AQXjaUX31h7RwkxBcNg/+6uLz+jRbZAUjSC4X0DRECYcSXX1X8HJpo4DEh'
        b'WpHFEAc7/NiSrGtIxsjGygvgbHLpPVip0IfB8mX8LMcjy05MQj41i30jFPBwaMMgvM/1swI22BDSODUIo2H9gS6K'
        b'nZ242HINMnLqo0Dhj+y6svlRt0fPzaHsPp4C8reOr4eTxnt3Hjnx/dIboEybY9A4jOiYwBpDhFYE1ncL9ugswJIF'
        b'IyWXDAKMFUi20A1WXfLleNW+Y360J5VHfzh///Ph7cAd1e4xEr4eIMiDnXs37RiTYiP14JuGZPvwViVVHrX4Zqdl'
        b'Z4vpqVRGKt7ZmI6Wuoea+3gK7O8x/PHzxXU5uzFG+2JBmzLLi2uu7M9DrLc8GeVRSRZ6NmHIYB1DtvdgGYsRX7cd'
        b'08dFVi0o/TB6NPQfSu+jP2lqHjrRxoePZTYqawyrr87RzMIYwMmAZofyLc07NhzVCjkcRw2GzbYaMMhNDybs2fTm'
        b'8Vj2T/O+b7mL8jo1LkhP5/xr+InWfBEDVxc3r1q+1T9MXeQNvq+tA3IxlY6l4kCuClhVrdEiq/TcR01hy/Kt0MoE'
        b'akGjYz80g+0pT/oUDq4x/99+P5XMS2mcvGscXolG2mJYIGzB6Gp09KNlF7TCdr9O2acY6lKVqxfCH+CZi01lxBQt'
        b'3/0qXVseEDl7Jn/MP/f1J/FbjbaMMVvZJltjexMycj3YNxZq8+xwOWh05PAbAuaEkcQ8t4vJym3DLlXswmpcfNJU'
        b'HhJOeC1jQuFDGWBYj0jpSh7YE1ihAIxtraq04t9kx+RVjeX9aWULahOwZkNJm63oL1zUPGhOmG/2a3xFw+FgMys5'
        b'hSankIcWK7PWPqR57bTxzljCbCE2Gs+swB+DHcrrm8S+7ilUpNhY/fNjPMBfel3LYWf4NN3MZjwkUAbSpd7ZDbzX'
        b'ztp0Snisy2E8D4S6YC3POKKb+IfGLohaSnh+lIeFml/CdFj5/GxmvihIELWAik7fA+4rAfY5a6cf5CijzN6A3wP/'
        b'BbIPdkEqlZ4U0z0/lB0UH3Vyfa7tcORbaEzCzGBXSveMHAdAr0TPR2qG9yUlO3D6Si8BOrw6obGDuIJFOgu48nx+'
        b'ZLtCftTH9YWMhy2Vyndvem11ZgVb1A5SPDBRHWvS+WHVpNxn7I3v9JFhbG9ieAoGRsD3k0hE3jw/1F1FP+ro+nLW'
        b'w0G3R0merSG6aYVd3/nwCEymbWVDplgP4saHuHPGYIzrgY1dkXTZj9ermw/aZB4T7S60d4vX1zQf9jRU8nyz5Lrt'
        b'XZ0B3VR2mnFAi+0pBht2V8yvCTWD1AGrNeKXvU2oCQtda3uW50e6a/Ekv44VMftcsSqDSSXSrd3S+483GVyvfULA'
        b'DuxZhwGyPxZvEzIiV+OQmiIfphlAqD4/3F2nJ30BLUifTD07/3h9cfblmOvLCYTwWgY7FkIizAbca1v/wz4sILCJ'
        b'Z0e5CWKOSId47FiWosxgkJtYhs5SOJqdPR7qYSw2HpA/hdcI7GA9Y6jZU8XSbOw+nvEv2pdNcGrMVZ7SsT+7jxbM'
        b'jqWjIbFX7xxGwMWlQ4nMY6PZMT7FV5qmu7c9njYNpop4EDP2dXRl2iIjmxQ0sUgg214wL/SYqtJn0JLoYgtNjE+B'
        b'L/B+6W73S1w7qqf0AnEdgtkC22O8v/ygzikptGVXgQrQFrWVmIOOOIj5upsX2NwKVio03pguZdoHRI+5xOrM5pde'
        b'rX0J7IDx+TUC+/WXn2yC71/8d3/1D/6K//nb//6vd+VnX0r5WbNLP+iFrSCd71RMQCwTJNm2DATooUAYaeg9JGSC'
        b'Ekm8IWRjViA/67poLiptAB/8A8rv4EWVzSutt3v7yNOXyvcUsW4ihoskHpOCo9SS2d1bgXQ8W7b0DQvVTkPdh3Vp'
        b'eixjBgjrcOwE7Wwk2xeI6yB8h8RfzOVsZGGwlCYWVI0+tcwuW+z7A43n2f014X96y/5jgm1nKv48FDxvuAM46LHR'
        b'7ezjJJ/Xx6P79XfUTjOyf80Ju7saheqtOR59V4iChgkD06jsem0aFin7KndfS68URQoh1xP+sO0R6RDbq/Pk79jg'
        b'dlKR5SXQisYuEn5xKbbkG+s6LSumGuhway1OM5NFNrex5F7Ej4ZZqjbFATmaGstcvS+21Hj0UtzPjrK+Tmi/AbE4'
        b'ZMTwanN1bxXSfZ0NNobOFMHqS8gBKFyam56vCYwW9q5tkbcEI4wABggUlbi1g2sh/OLh853QdtKUX4I0bcXVRyxD'
        b'I6a0PIQXoEor1V6qxYz4jH83lvR5jYYVGDHSmJ3G8tNHD0EaIhDs6Nh24pTT68T2G6zDnTDl/GqT9dU6dMNrph95'
        b'gfACcIHFYrH5lLvjVRumY0gvU8EWkR99FCTMVkNr4Bt21nx0aNupiDfmZXL9NyqWo+Gtu7c29xG7w5zJpiiTNnYA'
        b'bdw7mgsniXXMNroK0Vw8O9dpD035/PLo7L4XEHjzEmTjqzJZ2WwyBBl8VDC8NgJGuvXREO9G4jOVnKAHAZ0YPHZV'
        b'p2xEsNPzppTG5uN4kWh3O5+/+9++/94/aBp7St8Tzhe7YN6+Ur4TqhVjsQxLpZEGAlZjoEbYJWqCF85B9puHxjBc'
        b'ok9ASNWZUcCfwHVj8INHI3bdYfXn63G1R/W/Myr/a0T1pUXidiMlVFrkuZKczXNqSzP2iqwPwi7sSeRniQM8vrDp'
        b'tbbBruzV+QqVP+nKho/g4bj+F8YlrxLXPUegu5OlkcdSATQqG4TjkAUMqBI7XUfTYkP2BkSFUkPkzW9skgpyyPYm'
        b'xSRe53j8pYdj+p8Z02t03nnQHAjQwyPFiQw3e6BfBkk6uF+GCJu0MjZRuxlRm7hhEjJjb6OZ3Ox0zo9s2J/14Yj+'
        b'D0YUXiGib3hrXI7PV1/emAemkIwgIN47Tf3sCAopH+lhiCXZwH9DZ9KrrGhjLoRGs0OmNMyZoU9SfTiw/4GBxdcI'
        b'7Fu+Qcazjnj6HHK2DtlBuDnqLHyjZQaN8oC0lY7NCIDPOPDNNekE2cfEdiRN95W/8Z1ottSXXmWa8H0+f9pKQz62'
        b'8/fn914QBDq6uGSTcbGOwtKcGTzvozWZqsgXlf1pWoNu7BHE3bKzkimObgFS7OyzPLL4/i9GlV8hKr61ufzcDvvp'
        b'xnznHhyr8nFlKdPSZCtDfyhygQHexJQg7TcrgyyDq7Nk1sYZqaBXYzB2UEOT5ldPZJcZe9B/9vT099AT2Mxii6wN'
        b'jG/6zqfLdGZszevMDpQW5D3GyGsG3t6yrgeJhY31yEdYzm6R/R8HqwctaF9gVz3klCRKEG6tDhdDTuzdZStWXUOK'
        b'GBGUPar3Q9kyL0SkltQsVmQrFMSYzJib+K+aCy3ZwrrXi+u+fw1mBdFggbHhK1YS1p2TtnW1yh0SBZRjsJOIgTqO'
        b'nnx3TBqis4N6R6agKfx4ZF/9n4zmNSjFJ4Tx0/nVNXvA31673PXcoJsbMgb2P8Vg7BPbK1jo+56zmkTLhi7Q97QX'
        b'QhrpLdCsAurFtsyG8fhIgMAPR/Y/MbLXIBWjXL7/+X7fpY1RINvxCA1LSLBxMFMZwQXM0+TBRCtGMpjsFCPNpglt'
        b'zHrFim2FP22107mmpkdg6n9kQK/BKO46Dt1l7tuzHSw0Fo8DRZvjSANWm7oWdRQop5HYSBPsHiQq+EC1ZUyIBWIk'
        b'zsnL2Yfj+b8Zz6vwCciBj9fv9ofj336GaG2io5CJtVd2sfFUvUArvj1G0oYabEgPwLTazdZ6YNQ+fCw5GDAl3sDT'
        b'QNn+wr6Krx7cwTXuPlxZVmwDfEEvbGw1028jstGf50vxQdOKwfpZrEkILSxIPn7tnV73EfINeTKlx8m6fQ1+cacR'
        b'0Wb+3N6N9oebZLHZG0DYFoATqxuxvAa0EGvSwQA9RIdAFbuSoPtFaSIns4DOs5bbmpln6I/kiv+VMeVfLaZDW6Wt'
        b'AwmLwXxroK6g5S132UL0GH2LFWsvasSesls/3kHrbja/A7Thn9lreCSof4mgnHnVoA5//aFeWJ7FK2Ds9LkbVqGL'
        b'PXtCDT/t0Mam0mU0VnX7YsPAPoQckW4n9FYArHU2XgkPB/j/MED72wR4mMLMZmwFU6YDCZ+OV41+5I0+f2n4VIIz'
        b'kI42WCxDN+kR04ONCLtGD5pVQB5/IeU796oR3m/uQx30JrtsEQaGWSdwKYD4NmOw6ZxBNvFAguIcK6antY6qeP7/'
        b'5L1rjKZbdh5kj8eexCCuQcg/kFDxJzJT03uvvde+iFs8lhUiT0KELX5EIqV9PV1nqqvaVdVn5kTAH6woCocgpMMP'
        b'pPxBCigCoiAhICHkAiFycBKSGBMIFyECcSxbXAIk3BHP835fVVd31/fWme76vm55rHGf7uqvq9717r3Wep6913oW'
        b'OynGxE9yzjS7npPFHcSezVAsvH1sQVHDCQRAhjydDqxNtIiFwLZIxU0zG9jssGwNYTeUNld95Ry9koAJ18DgskB+'
        b'rwZt533dBPdlYnkB2w3qSqi1gmdUV6KDtwBnsDUYZIOyPYHtLcuFN7mkD8U2+Nkcxj2wQHoQezYL5DNvVUcWjolQ'
        b'X1mZLhUgXDtyLuhuH3ZQHQ/BH0DXsxDaZakuhpJiQsYuXspug/40DdoHyNgxXMaFyHG0BWBp6GTPDfXwlBcHE+xx'
        b'Ahn1xpoopFlwZoRz4Cp8QwDE7srIA1bpA/stHsKc7YiZpVU41lySVM8SWGBBjs0BRC/TlJQMZf86Sw+BqLIkdZlr'
        b'ZFOBjYCI0bwhn/m6PWlfmHYpLwQ4uvht4/acgkdJrddKSX921oYhHBYHMmUGCJWlnuSkkqQCv0+DJUpU0nYzmhTM'
        b'QBRfo/LLdssHsWfTlE4Oz/uACBsADDj+JnR4UwCQcL5kk1Mp1gAquYrohw0GTmwBe/H9u5Y8QPJ3m/MnYI7bB4R4'
        b'ffwCtZOPkosedN0XE4OCURA0BIdHbtHnybuNbtOINgClUsCp8Cy2+NJJ421aOYv4Gdph92rH5nQv5WW4FPa9aNYy'
        b'FIkEqYQXUi3i2Yu3yDiSTOnSS47Zm5DAZgOor52x1inwnZUrgJ+lJbIfvr6Zm3t3WTyzzegg4Sz2rmo7/NsPINfi'
        b'jcX2QtDKhWPPi0EkYzt6l9g5bZLHEdPkYu36Gblzh5lOb24choevMTm23+UGBhGU7dLVDtAk1vXMKY0HmMihqSaY'
        b'yFI4bD+HLyvDWRsPWOQPY5E9nEV6GIvkcBaFw1jkDmdRPIxF/nAWpcNYpIezKB/GonAwi7w5jEXxcBbZw1iUDmeR'
        b'HMaifDiLDoQZ7DZ8Uz4QT516ro4SdNEZ3xXgjtqNEZaZoamCiJqc1TrOdvcZHKNbI8nlHK08gIL8oTCDP5hFB8IM'
        b'Vg9m0YEwgw0Hs+hAmMHGg1l0IMxg08EsOgxmuFGTCGEppKxd0sS3AwGX7qPldBsXWa7BpktRKpENnteXwbEBYO4p'
        b'2lLKdHGOvn5kr/vADItk33EfE+bAqo8uxxXVIK+O65gXlyxl/vZtmZSlUvYQgbEmtSYu+9Gnmx1JSBObOGJhW2aY'
        b'Hsy2DXEhu0WPfpY+Qu8lPWChfU8Wbg4hyAWOypjN1uhay1QiHDHObKisU2GZdUlzbq06VWNnmzXWQZkzdWNweJFb'
        b'uVf6eVoo+z3jWuoPefFCcYPbOjD/1SPnOzU+pOG5U2aBQKgmN9/mZJ+a9mZ4O8tmYOAOfLppS2nkFHhZuHZGtJjl'
        b'DmnWBi4p51foZpDDyPhN5iCYnrRmGyl8HSU2AZSSXr3AD7uRGagGCu+M3JjdreClP0izHpJuf1ncK4heLx99+xcc'
        b'HDAuXly/XpuiiF74YRwRh3cNWGb8ZGlUKlJsMz1xhnd3rYvhxG6gwYz9aLFIxUUpPiYeHO9+9j/CZ9e3fPbNMenx'
        b'1em3r5/eVDc8K+enEytxK3urPA6mDqkZFMB3rngb5pzNxeJ6HlkmvKSM1CKl433lJVgMw4VOvd/heNK124A/RAPC'
        b'WxqwefLNPf+9tUEuMliHQBWs3JdhLynXFkuoIRlxrIF3pbjSHKUR+OzDa04+UfbPIHjbHB6ooNH4CE+/uM7LP99o'
        b'+d8eOd4WLuTMUamteSs8V2zqWw9SdIwcJscpYltFBB+r1Acy+DkjT1ei4RDVwTkitq4cnv5RGpQewaBd6lJWwjK7'
        b'vmQkSc4iiL7wULtQ9ab1yvZGZdMBJSZddiaFZkeyAyQI/l5NQu7wKxb8B7Qgv60Fr5SO3GixtdNxtfHw2xEnngnE'
        b'm1EkpZzyHFJNS6LgacqCYVMQV6NQ0aUgo0yJwnljreKz1TYbvcsrRvw5GBHMoxhRX5yedeyoflnm9fFuk+yi+hV8'
        b'9z5kY7I467qj3niX4oT190YVHDR2i0gbgH8ML7FaDqWmObwnrFt39GAfw6TjNwcLBKOUxnABDh8G8twwvfTo4OrW'
        b'AFZG6QX7vjReic5I/SDe9CAtsKjdDrEJq7Zyi/07+fRrwhc/8ut/7Df95E9st3gOzMMG4GlIanUi2s9REEFLgMOO'
        b'6SgNLnOqcXWoSxEvMbTKasQ0XYsT/rByBf2f82H2kYe3CeFOwe/VE3P8immB2g0GW6TBKZVCzrlz+GyQOBybKwW7'
        b'o3dvI9titc8BHF/wkzTgX2DfJMCn3ab9bpq2JkTRL9pStPvtk6uz0zZO+LF5drENLpRZArBrOQHKFQ4pjQ6/AW4g'
        b'tRBEHcQYx3LrIHjJaSJYNsrhpEBxyOAAcnUl3v8CH08P9Obt8Yqh1Jk86gocBzSXARoiXr2ZhUKQzrGfK7WCv0Oi'
        b'BuAInO81sA7eYLe5RpWDaFZwxb9GQ8MD60AhEGDsUS7b0ycmnZyyKZekaXHNk+dn5Xx5WhFWZR5Ni4UBE4qqIHuG'
        b'JbjWA2ZXGcNSuo7DCpeIrwATpoRinORIuX2kvb4SLn+RTxsPtCxy/D5NJeJYlRgFYLkbIF8OmbmjgXL15vKd+xP2'
        b'qZ1Y8/zi9Pz6pUUgtUCww8OkmrUhULo2M9LxAExtlYOD2wy5ArE2vNcON3cmA9ci/FnePudU1pox/jINygdaO3e8'
        b'086kZL3AVPARIEGAdBsSfhwiNcUuB13Ldk5ySATvha1CxXg3Zqy+KjN/1Qfqhlb1R9924QJ/z8aTk6V0cjFrsSgu'
        b'E0ED2C2yjuNwP9B5cNtkOpCI85yB3LkFc/E1WdtqDLMOxgjqSWRfs7j1WBjtgRbOH++209qlNKLWYAO1IJtGbMio'
        b'HFNDrSCArhKo7KvgiSxktn2ESaHc6XnmJE2cq+vNJ6vypO+ycs/K6fnp+Ucn9bKct6cnP/VivNhoXOL9A9H42Dg6'
        b'ITs2Pdkp8C0gidhgVW+cNleCqhpWt1jg406ZDarxBS/YmGEl0/4SrToUiNDjdWPZDoAl9IwuOfKYCeh/OqxOGJzb'
        b'CEydoqQw8cOB/gc7VkDyKRmVzHBOp7HhgSqdNQXTL7iES+TYDFQp9WxsM/PzywvOI1nsWPSZo1IEnMcplHLqiIa5'
        b'+y4WzFOGIi8UU4AzssSI1eocpeIGeJxO/KHOFfzxP9COQ+GPcPygvUAWVE4SbdiTvMaazouKmBaHetDoWU30caRJ'
        b'iYqyKeubHJURwePwHlK3duWo8F+nvWsw5Gvlo0WT7Oqbp2dnVxtkeDzmPAXBOb9e2mqe/MSP/4ZvfGOzPMskVES1'
        b'aibSLzZWlZ4RAHhGFl0vzN7K0cnGZhaXt6QhhWUYLIjOqErR9vX+z3goHBKP7xi2lCEBa7ClwaflFjHDKdigISVm'
        b'0NXIbk+wVG9rwmtnu3VvporB/huNx54rdeR/nIald1yHyzHHJb83/iloaLnmMR+PCcBFN4E8cFyfwYoLokByCOm1'
        b'DZBNS8AeBo/6WjdjcPJVih5hIAczJVGHdKklCzOsXBb8JVpxKKiRjt+Pkb8fRq7KE28tvL48ZSnrmzHPPXntDBDA'
        b'FZsLeycoZ4tRegqUycDlPadCtgy3AOWznsOvZBmPTFGaEodPI7BRt3nJu5/4/+YT28c7w3wbEUq8BHz85PT8+Yvr'
        b'k6flvF/MuZEXzcvgkJi8gj/lKIBRDRm35QZYJcYB+Q47iqckfS8hIBYCSHH0fC8TeAR+FudKiPt/ab28X+vr6fnu'
        b'N7Dg/1wlUcJYedrbq0OQbzyjNn5hOSPoMEhxCPAE/xM7AvRTNCKKsnA4rXO35N6v/TcquYGmIrYZ5GLbfOW2rikm'
        b'G5wBhm6Kn08phKwVH6Wm42jim5/edp6Gjwqv3W3qX6Op/v2aeg5f/2ScvKpYvWxxWhKX5KcNMHmWGqha4fMQgwgV'
        b'y6jEZClEuLRVTqXGJ4y1NSQwiBW7/zrt1vdrdx9n5dPRT56O0sfl1nBWRXOoX5wz26GUPXTdtxKAKGNvwNwRkNrC'
        b'/MhZTQiExU+OjzWcRDVbGjW4FcP/dxoe3q/hYEnj8pNydnIzFH6jJczZvBTwtt6Z7gv71+HUCGWACy0ZP3PwMSAj'
        b'dTxOTTwIVsBzpUBNkenAE8cKEvpfaXl8v5ZzqV+1mprPbHw3oyCgxYifG+JA+Gqeezlp6pyoglXF9oZzG5+cy8OV'
        b'Hhp1kHjsvyaTsTh4+iAy2V27hZqLHGcXU+icIEyVBg57Ra4Sx9FW1cLNfZBqYa51iboMjiN9KvweQN07u2b3ksHz'
        b'B2H35fhWuewn+PJZ33g5sDF47lGBrZUr6+G5wP2VWu4kyWV05Qg259L0vOQwNuYxou3TB4rcJAS9tQ6d/wXWZ/N+'
        b'rd+azfuRbWzz1LTNnH+XPcJ26z6HGCtLCGWhpRGxu0rpiGzqeAYJnzdj0X6IKeaiMa7c2/4yjX7voO366cZaNbz+'
        b'KdJrZIlRbb6IndY674tz8OYolZfsvQ0E8RQBuCuIKKidsMiInSjAbDut/fpXvpfiry5/ODgNZPrp2G7zJ6U/O11K'
        b'XTYbfiHlXXwsFGjKEzwcL8YruESzwCogFFkAWimYw+WWZqK1OUxeVbLptpts12N7fs+IrVw9H+cnzy/OTts2tgcO'
        b'lz2KfWbQ3y4V+LuPucz2GoVSJwaQdIQCh2iT+c4OxMU2khQHjIdkl2WuHLz/X7TafzhWn2y+zTbGReGSt4Schg3f'
        b'rTXY2ba2lioYS6i8dSgCCJdpbXFIZb3xiB5b3wC2zjhyWQfp+T0iuB8tlx9dfO364tkZh5WzOs1Vzg7jFVeVbAHJ'
        b'xjQ1mkkFeYB2zt4DfgMbNangEwonaEAsxWdwltbXz0VzeN+mnl20b7LWUHku2jlZcIaASO6xtdlgOiIjdGUFS7ah'
        b'DFtyamBoUpUyKtgPlkqVFexU/QPGPiZMe6fBOYbn+MYDigN4dbAMLZWdsqOXGKkcUCxQWLejqskRH4o6q6E+e61G'
        b'XBWta3WH/xONTe/d2JdjdLwKay6cknm1xglPYBcBW3mYkq0bllxMDbimYpHdSCH7Sh06ljPw/AUYtq2c8f9VWpzf'
        b'v8V3h+pY5XDcwlPhRjlECclJTx2QbNTYOSFp4A8DAKUhW+fKtvbk2U+ZjFqOT5rNrdrsjfkAbF50Z7aDhJQiGLYr'
        b'ApPmZFoZjjMvko5InQjvZ3IcLE8txZKwB3glZwSbW6S14nrtacWN/2fabN+fzc+ffnrFzviT84u+WWa3oRxLFXQO'
        b'HIsYErazz9N1ihUinGnPWFzY2rCVk8nS1BlB0MYWMN3lGdd0P/4KTZb3ZzJPuNtiamK52VHmaSilFgIYRR6zhsk5'
        b'XtEYGYjL2Olg0QFbOjlErF40uwq2UU3o3QGorJIrb9yeLYV1V+P86sXVYutStXfSnhbgze2B0dWmlvoJGcbmkHAT'
        b'tbOBsx1VnlibImZwwPUY2fMw24vgd8W4lrtTSnFEpGHjl2PBDHQC52eeXoNdv0Tr/UGtf17n9cKkTl7KbW6OB9nG'
        b'TW0EDdjALEl0NkSKynuDzTqQsLDYtvOQHL90ScjPSMxgX4GVpuqozN4f2NS6Z2OvP30+rrZbGnv4yeXZ87sLvolY'
        b'kX0iR801R/FJqWIUgQgrWEodcZQI2IygNgPAV+G4qcJ6lkB5aR8iFdE4YnuVM3vznk/GOHjqLgCxeGosbwB4nLn5'
        b'oIbiMEMsR6BOS+1RLcAj+IAtRSjbjDfTzBgKljlm19lnWwEg/yeNju+fQi6DxV63PvAodLQ8gb0At8QDT8XSALqm'
        b'B0UOsFlY4Z1CCqYDcvJAIdSy6JYyseEv0wNRLL1/2zeDot+wfpnAAkpYW06DR9u9BjMVCFNnDKAQMSOcIyGH6Cpl'
        b'nQYyWsgs/anqQJibW2sI+Qu0/pBA7HbuH3gACAQFI0u1EXCxZ+oyRwTqAIqgmbgrhyYjmghcUrHahQdhIcoIyprr'
        b'NbT1l2GYfY9o63YKIM9mADfCkEqtTBneINd0tyApG4AyvNUJy4FBgKqDJQzLJUuKrgCFJZCmvL6A9pAIq2/WjxOp'
        b'4Jex9FI5VT4BZQBJBA0Aj4g/k8pWApokPWSNnnN08NXmYTT1T8VWv6rs/F/QsAPjqHn60TbgBmaZAqQ7K2u2tDWb'
        b'Kck9QHJ9KC2mmSni3wAqYjEgCAKciGDUExW+kH/aGkj8b2icO6Bxr+dRVaq2H7UMD9NgsAVr7gn8xQ9kFySPOZyP'
        b'ziZOxAiBrTWIM6HkEpZZnRmMQGUFHv1XtNAf0MJn4xp78bpsvA5AyLE4FFQV0Z9FkCN2YxOIS2omqTGhdEJgK5xF'
        b'nQD7KiXMeqGupm2IsKA8q/VO3ur7J3PASBuDE9tZJbB3hUcT3UTjnHGkqqXDG42Yyvkm0zkEICw33G+A4Ax8GX+Z'
        b'ebS+Xt3rbXhsez8Z5/3i8snlRfvmVa93w0vm2QPyQzDZbcp3R45qAdG7HSFXoDqwl8KxzNGKHw0gyNlhR5tSsFXV'
        b'LEeqD8TNeBCDbiZNbkILaIll06BkxTc12JG85EI0QRofnq2BNdoeQ4tdFYwkpaoVDH0gq9vQJ9BrWuvQ+c9oWDqA'
        b'YTfzMzk2ZJsSeLPDQBgoY1g4aSd1W9nXljqoRs0gXq0mYBSkicqyZdaZ15Y4X65xCptJD1iWD7Nkp9cDvnhxuXEu'
        b'u6QDthjWPHqKxgN4G8DG0W1IWEXPU1/WSMHaBJLRY3d15pKUc5I0G06eW7dMzGEsu3i+lIFt2ZSPLOaFr5WeKXgd'
        b'i1KumzOhrOXAbBtKoIQbvmAy1qvNCA8E2WLYcCVgz64dADENiD2MZc9Pz89fVoNuV44L56L1ORrkaMCPGWU2E3v1'
        b'wU0Djp+KY3H9qL6Flh3IQ2eq04hA420Lrq02PHmRA5jXLs5ePDs/2TQvbkuCBMwAjgQP8gEEMFlpAYuEwGKLL0jj'
        b'E4ER4KXHFl3LbIVPDitXeAkROzLDMKvF5V7cAUyb8/TkxfXp2ebMKrJ/kVNm7TB8Xs7HdlOSQx4LjtO7IgUbUm+l'
        b'UFN5NvxvJipbahJJHRTJrha+ePHvn94v/YKvjJVOElkC7KQV4BOdLU7Okw46J1wNJgOghFTZ1KkZYQgsQjK2qw78'
        b'AS+iI+bUlfX8/2i5fkAc/41XsIzVFtC6PCqH2ILsuYhEoqzMBprJjqLLqWZ8MblYPGeglVK69Q4pBPshlJWr8u9Z'
        b'rsq9hA+K67/xFjxnGyHDkN+aYEWVN2Z1Go53a9WwuQBrDkzHUZFAsCNUMCug2UKpf3B+CW61EshL/DDumu7MkFc1'
        b'cdFSqAnIiGr2ooX1AdYAvy/zuEE5AO+ieELbLnACOEpsTgBzbZqdBXCregReHh0W3b3+Xe4VKAyTgYr8rMsobRsa'
        b'W5dmB3NMhjMWwA5lTBNG87FGo7yBkG7YqjXMCmD9YzQg78eAzaUuoq2yGsvRw5gBTUb24+xHjvPNMzlQC9t5HpOV'
        b'B6tUprWUs27EDIkHANHb1WJ97x4T5nzto9PrpxcX37xiN8x2VhPZO4c/xjliMIDbKTjANLxxkPI8ETFmAWOXAByq'
        b'4EHdxRw81wc80ADjVM53Wqmq+n9ohX3/KWTpgXkzj2TlNCdwJHhOtiX6Tl13jgeags3YbJmOh1CuSOVR8jAILyD0'
        b'08fSmiN5BFTaHUa/bxNGnXxAqeT+VwHc+tUjm7AvW4ytpFEUu7QimGI/SxoSFPHFOoTPAgI805iD5MWXUbHtzYij'
        b'78Z+X//+7YtwH1Q+uf9VqOF1/kByAHpINgDvZs6ZrB1QKo8I3F9sKtwcIJvwexdcxyuZlcFMJ37p660C3n0AqGoZ'
        b'Nvqm9RyySa2Y3oH/QaE50cH46YsDB2DjKYfeZNDPWEFEY8qVcx5dZ/96do0syCW/O6l+/cvbrfAhwasd7yKwboeT'
        b'wAAWZ2Pb9CTtHn7YjoSV0nAu8LTTSTGODoP4z4NPEHk4SVFtc3eI//oPbN/EhwWydrwLNdTgiVhmUdrfkQeCr9gu'
        b'Zc6BPACOnzwstzZVVnIihyfOURsVn0/BsKBvHXG7+P69YuJfXz+9xy1kETwHLwRRBNj0xWOfA4bg/xsiBeDVZMq0'
        b'qclMBkDTwT2mOJhfOo9TC7KpPhwh0wfkFrtexjLRDItfEfWAgBJAWzaJSjLesiudIwSV2NvmWRA6k+cASIO/TTks'
        b'YB1g46E6Xe/yB+UXu16GcpTM0nPKBpvSuqvBcSRfxCsBFLStTs08WJcyOzZBlwqGJsLh4NgbnCrm1tOFNx+AY5zO'
        b'e/3CZZaOBA5lkKgDcR1c27Ev3yNT+h4t4C/wLkKAtwb5w3KuWDFzGX3HLiXTon0wXXj7IfnF/e9iSReaCpY4R0sx'
        b'TI7Ca6bz8iTgHSxak7GAzgB4egXehEeAo9vGkXnFgabpg+nCy4flFve/i2266BKrOvERpM6lDqeYbL4LNk4gqRCN'
        b'TGQKfK5nng9XC6TFwg1dJlk+4BXu/XvF8j3u8QqfOagdpBwJ0PXauo2IfZ5cPGceUCVAx1ZLKyO0WZzonOzI665z'
        b'jkkuEsYKiLz1Cv8BecWOd7F4BadBxsKZJk45TAdQISfrkTBjB1xk4ylb9XxqfoxZObQUL8QbjnJquqJ1d+sV+kF5'
        b'xY53sXjFrAKvmK611IstOgvotksR1BqWqgV0qJNTNZElvVHWPTRALe6p0tZY1nLCvqbefPnifPMGQpVjV4Y5jimx'
        b'J364Y/BbkJo+Uut683bMbd/h8bYTbRmdjvdy2l4ZihwDy8VYBG1dA4EwUQGCHQJfjImXwwGuH+D1HjkwSRuD8qvd'
        b'NBUqdykrKX1Zu4L8r2lafDzT7GZHyLYT53jpxDnedOLgD883Qqa3U3s0hqVCUkGGAiIPD3x87ZYFc13EpJg5xRXZ'
        b'zgANSxkc7uPsTM6EwtIqFVvW5Gl4Z7wm6nxrX43lmPOajvHO7XG2NNIBSABiGs3m1r6bXtnjTa8sbbp+cXk7h0iR'
        b'jgw7bZRN8YVVUKkFHiBKdzNyDtTgODVeJpsYLKg+r8mDURtrs5zjlcLaJfifoUH5MQ1a+mZgx3lnOec43vYNbeUN'
        b'ckhCya4oMMYa0wFFsRlcpywctVkSNU5Ti5MNkZ6z1UqMzU6WPkoWfK6tlf39e7BnTdEZAaBzBvDluHrxbLx86k3l'
        b'7TGj5PGmgv54XpZnPIG7uv7aZat2IwQn1DgtAAEhRZ3ZUAA598LqY/xcb0PDlzU2E7AuY+DPTR1yZQshTB4xtrh2'
        b'l/Pv8vHtuz3+Jr7dffyt6KTJnIUgnO6pFFhIFAHFjg8cSA9U3B2frpo8xUpBiPKxljTxJV7D1KjWzlrXz0LXlJhv'
        b'9xIjO2e/Hkc/zXEKsx5jDzRp3rns5aVZTAmbSaUda/AJ1sBbxCzPnqQkI00KQAC7CVwfkcqB1Q7H8XA9tVQH/NuG'
        b'qPCOytPQbIHlAGUpXrXbij9JK9xjW3FXaZamCIcNAnhymnZKAOd+ZGcML0EKPLum3lrrVEYAEhPkWUm1xdE4ldpo'
        b'ZP1384gJI6T1aLUmtvzupmw3241ibmab6lENEtNMHg8HkKiJ+nvgojxoUQXALiWzdyq0IS1ZClBV0yobZnNtbqyU'
        b'KP0BGvRFIIS8ASFkeXYeJd6/FjnyaDXw53tHiT1dqnJNBWME4JmCdO97zZRohStX5TA+Y3Lss2FbFWAEV/P6Zc0X'
        b'0l7+wo/+xrunAWa4qsOmyUu1jNCZEZ2AZ9gBAheXyLYnsHwbGwf9Yl0UqUUUGVMp6L0q6+W/kPzyfQYsp1V35Jfv'
        b'/Pm1hdDE6oMjOABrDzkKWjw2JVIc6GrOmQPKEUx9sTb0yrktqQ0LHoKsl+JcQnSWFTv+FO1Ie7Pj9VVhO8lRAVfy'
        b'VTySV59RsvNRAX5G45zEWU1HYI3sFOc1YI0msbANECZ08A5daxb9GVqT39KazVnJHXPufuH1dXGGg7yB9wFysWMA'
        b'B4vp1Q2HjTcYV5GdKwUcl55vZ0c2JRsXyQ1m7h4Eeqy0fS9I5AvJML+tJW+sjCVU7AZMz1JxJiBW1oaFQixqWJWC'
        b'7aXdT+u7AQPsuUZPUQflJFuKjWqyuu4vb63BvND1u+a8/PNr6xJjdHmZr+AT8l9ohcXLVIfHJmp1CmITiHtdBL+x'
        b'+wQsrgLJ+AaSEpHRU3GmxHV/CbI3O+71lzydZf1W8RMANnXEAuQPpD8sREQaBI4HBGb5jO08zZ8psl4ISX66vDqv'
        b'aVkVtzcR+dtVSZqWSbUSDfvAI/aZgI6EEv2IIJUzRpJpJ+QmDmZUVt6Z5KKGqb5NDm6pD6yK358Y/hurstDGBuiR'
        b'2Evqa5XqsHXY7D+RI12kil+tznd2b2B7cZZoTUwututESlrZY/8RrdHHBCpbMzaNoa8OKmC8PIoemXwAApdABo/U'
        b'0WWUKiqje6GOWh9gHBQKNgjVMGs4RHEOHGYlj31ghz3GkIXL8cm4vBrHF2f9mMq5XJOLy1uhuKisLiHYncVgL8Bx'
        b'MmjZMM57q4alGrZV5MsIzOxrnF6pL0Mqws7fMLqWZFblIP2azvMuKtIunn96TLWQ18ZDWMKTIkBPkuELIfUUWQkE'
        b'Fg4uPl2yYp1GSg0IMovBI3I2BOwYQt0U36SvpI9/k4/7tmn9Ze/hMYx5xl7irdY9XvNSkVhZqNUQcyJ7/xuAVGPX'
        b'YSr4cx2mjlytrUDvprsKM9hG7Qq7j93aAKLfz6d+2/R97xQOsxxpBrDRIrzx5CQhUcTKREVppC8W5TkQI40TqcxH'
        b'yqCCO7GRh5fEVYB2dVWs3Me3TdP18rR/NI6fsXN5O6DcJM4MwS51FLIDb661sBoMoVNn6b205gV4VkAt1APh6Rim'
        b'Z+WVRZXE5taVwPJv8Gnt277fTckXNnP7Jv67GZADDOyoBm8ra52yAByAhdVi2dsEXjZltMaO46i1xwp+g2xbq4li'
        b'zWS/TR29r9Xf/j4+srzbC17mUGyf11oW59s8KrtnB8dui+QcNfG8y5qOp5/FmpGA1MBxkiJBVmlzIo4g/YLIubUK'
        b'qGVDuHd73s0p3maEGfV0CmCKN7Xw+LW3ztJ0iga1MlJgs5nthZPqXbWSQgGTnFZBGAaSTrSy1hz4Z/m0+5huyEqt'
        b'ct6P2Trw6d0hLJR9Ybtjm743G9i14iMbdEDVsBAlVXikwyK4FEw1ziI+djDNGAt2+gROQ3Jr6+Xoe5EpfnW0ya4Z'
        b'M5GjOo4yp2qkNkerVDavhSgGZD5RTFU4mwXkh6m1OMdb5wLYbfrIeRo2qK9U2/95mhf2Miu9XR9fvLjGwr0RQ/0C'
        b'qe2cobCtsXXjDa9CxSEPhQpncjECjFbAOIPEBCJAM8GRzOwsvOkIu7tt+jnatBfp4mV17p+P5Q2HS/G2omsaAjYX'
        b'eubNp7YKlBY4QIWqbPiuWEZLBZ+JFF0LgCpYKD5JqeZ1RBr3Me/w3jTneQ50pBaro95zBBApjRbPRkyXnDcgoN5N'
        b'0zj5vcwgYwaOhoD3cfTLsDKGW9Xd8nuRL37Vq5Zv88q8nS1wvQknr1jN0+el0aqBDEVqHWQwpMwq8TAGYnlDtpzD'
        b'+EJ1EyybclZMqyzroRgbw45mXe8wTua9WX3DnpIx1H5gq5EYEgji2qQDKN0iomQP8yyzLfJADRPUXcE9EFwSkIGN'
        b'HQGjzrWGsl+mnfb9re7r7Crw7hXJuOEbTQA1yYFpAGFIu4k+TVZ25qrOqUT2jCOrzx5DtCUXF0tqa9Og/kdaK+/P'
        b'2jcGq23EpwQ8twKKUtrBJ9amIpkPxZcQlCZcu+XKOX5m9ErFuKmUiEAE1gkYu3YJxFvIh2SOr1607aSBjdEAxcfB'
        b'23EcXbTHOZR+zPseZzhLxugdo2/+4XLJenF5ev3pZpAiL6K3g5sMF3QUDhtIAtSCtKiTqjXDugBvtQn0GV9HUoS/'
        b'grPxNLxo9ZxYhWgmUqWvH/o9pG381vadvTgvx8/K8w14iQjpR2DWA3RtTmRDMDS1IMeON3WutKSug3cCKQKRGw5e'
        b'wA9BxGWrfKYAfljTumfSeEit+N0subz41s39MPYZQvoRkkLk8eUQAGJEUYtoCoDZRR2QOpwpR/ZkVQcPK83zWE1g'
        b'bV1ul8PamI//lNaEPVlDAZljxPfx/LrgJ9/FYJooyFBlCA9k1HoeOhVKzU7PlUhSeR/QWMoxirC8CzyPTf6JfKth'
        b'iwaT1/HKQwLD72bXJ+PydL5areCisHcq9zC754gZB3g8sYRzOgm95BGBXAyneVYOtgDfSkBdpFgs3pkj4Ofk1bF/'
        b'/iH94Lc2CoTnmocj7eLycjvReHEnrhPBh7MKMxDSAR8p8JNKRWTlrcxYlLqK+tjJfizC4MRSZs74BCErvq33Nz4k'
        b'DfzWJu08I9FMrxoUqslgiwExzepQPznlsLZsEOItgj0iAr4nFXG7ZlvjtLWy6hChX21cV814SPL37aP5luPfse5y'
        b'vLi6LcDIgfUXhrNhycgkW688rMizgOT3Wn2mlJzaCZJgSqvOAqh4D9qqSNtgOi3kB249HpL2fXvjNsh500R0d2xx'
        b'UGCkBKAsVU0pIDIBoCpSJG14FQpRTXyPKc6y5wg/S2vP2hg+2FSxzq8fUu59JHs206WRY4gVRUv2iodNoAKs8AZj'
        b'ASDOha1ryLvYiaMaY5xrVCToYANIv2BqqhwQOdedKrv9WsSGr9e8yhJC0DiQGEQ2auh4yxboUnjikSj+tpzbgVCD'
        b'5BjKrCP2B/qXHYBvfa7tO7bnZ783p8L3evEcnsROydOz07tx3SPbfvWIoiQZ7C2CTjvnO+CDMw15qmiXWdmibpCS'
        b'kdG6W3pVBgs38FKyAyjWuQ7+8r4gxZ2pMy+/yWskzbFXL04B5p61lB5KM9KxkglwFy99As+baEr3kxPCXfbRS6UQ'
        b'lq81JOTstlax9RdpX9jvfnxpGoDgcySym+BheTePfJwR3gD/ON9TEOx5ByG++OgGmFtTpDVkrdQzNqKJE6CJYsCx'
        b'iYLYuAds2xfQ2N4F7bLNI12Bcy7yXLUDVmTY4EEyp4L3OOtrsSPBFmlZUqOIC/VaBIDdgzIN6/3Q/oDH7Qtv3B0t'
        b'hPCI359evURSnPlylGMqBesi0eTUDZyKVLnYpQ2iImACOlJprxc/eGukzqcGGEmaCbdrD1iW9wp7z/vxR2zHftXV'
        b'JEZqYXTnG+h+H9lEXhJ5b2ZIjtoX6jhrA6gE5IUDySeIo0cswfq1CK6F1TVtFdCr2Rf0uK1M3pa4LZMWb5ZMebcX'
        b'8JiW9RVRYAfCPOuPQphODRg/44lOmVOidixcZq0MK/hAOT1ljVeVgtTYg9q1ZGsnrIU/GvjXgXc97FyeLSaEeO+7'
        b'r2l21RIo9olkF2U45Z2bl8IiRKQKNtG7FQb2X9Iw2b9hFw0Ysd/A+9u53os6bS9UngUAQcDIi45Dq4MdFY7pvIyR'
        b'mjgwtAFQ4pHqErUw7eARJQhAWD/qV+Peg3WbxXPCm4yJ0JFDBMBljS5PbzoeKlRKe3CKz5DaBzgAqLRvNSt2Z2w1'
        b'+Vw4izu51aN+NX7v5t1c522XbKkCcnGW6kvXbgGueqoDBBOQCjm6BJbGKScWAvxSELD3wZG1kqqbVGhYi/o/R5v0'
        b'YDYtC8WhsoG6DWoTNlgwozpv6oiUT42LwqZS6zB3UywnyWl2kqiO0Pn/CDRgOz0+sFBh70Ytv7t+STMDh3B74IzC'
        b'm8kBrhWApIAPR6qIGDPD6RJ4c/ag14L9lh2lZYCOm529u5rdAwsVD2bTdqEyFypzvEAaWBbD0W/OFyQs/IZdetQE'
        b'sja3RcvJgMlk1uIjjwNNFRidg9TVMw41+8IcrZxfnFOJ/Jhw43adlhF2Qj654Pcc4C7FDdes8lwD7uM5yhr4w3bv'
        b'SgDFFOo3BeW2C6H52uoDey8fyqRN4DPR8QrXJIqe+NgsfN93zgqYJJiUBOUYBVabA9uzkzoZgPpkY7GdJzfgamtD'
        b'XpiO7b5gxvXFxdnmgv344lvnS1Y+L2cvzzeWri/L42hrAP5iQHRrEz8CGbkC8M4UJpxIUvZFsJBhiO0RTKw0x/p0'
        b'+wB+svawhi1LZoXzxbsW/KvRuqYMlG6ijgIkxStAHepaDuBajiONE+J6SQ74MICVOTfx7+xqvZrafcGMWQCaiHZv'
        b'1oiz0keB0w/YkUR44OnZkTs0sgLdqICJDMb1NkUK1wdMzI4+tKQYER7X66HVur3bsjmtSTmQi0zRWPBgAH/4xuIy'
        b'9a98ycioS09mtZPdYGI4EhhsqyKI+5FcF5uCxLEe8ey+MAS+yQs22fG05u4CERZ5pKEhwLXTq/CtO3FLXOAYR9Bi'
        b'34oEBPRhOQ0MhCsGW10F2fKVp4zrmcnqoUy6XSdO0UDGWfzBI8li4QbnuUkmc0S2SgrSJbw94PT6DNIopnE864zF'
        b'2cKA+MA6hf2x4c2F4/Hgr09vm8949GmUCiiDx2SIcLAFLgO/6TCJ3VvWqVQTDZYqgoewKgr+1FPCH3uXtHr0qTbu'
        b'3aTz0/OXFvHo07AdBfh1eIolTkuRXLVlGgWerdZraUbHLN4KGDK7nzrpMnMZ9musD1iU9m7R9XjNooGlAR0yPvAi'
        b'MSEjNfjNhC3gGckVah63gTCRQIsbJRVZdsxzGJ6yrWkc/zwt2hd6KJcfveAl8PHl+Hh76XNTUJQ47MCppsDujxmw'
        b'HGx2Ao3i3HNbc2kI4pyLGkKk0C97cxrlOzmppfLWu6zX6qqszqG+PfT6IkOobaYOOgIVVYRyiuwqaHiE2kzHDhvY'
        b'XiF3iZGacF4niC0iBQVlZplhduEUQLN6RKuyL1iwcfltTcL4Nqu8tsU0V7elyJkY1UlIY4RCSShrlX0rlPtFPIOZ'
        b'QHiG/MIUuj64IDKvEccysFQ8UNJaxP5l2ieHsI/f6e4X7i7vDR6HM4HiilA/rY8BwMBRMLUIO/Z5v8OuMMPzzJg8'
        b'0i5+SJ8OmZYxJGIrWgeWuI5eZV8A4kYCHeDuGt/l+fOzcVtn4alZd2RB/0zuKatGmdoA4ziA0xjgChcdS0iQmCVR'
        b'XDE0KY1FDBkUsokC/bUHtukKlrj5VxuT4KfH1ad4HHuT46y+HIOVlsmBtln7jUn+CSDRZaGGJP6DzXnje8efxFeg'
        b'uXUS2QtjGAc4cBFrBueKAHiOMrGIKeAaJpEPs0AotNpyrQZgzxlHQLiMu4qrgu/6oHzquDrtRAh3SniBZ8Ix8P88'
        b'piD7MWyUYwAfNhBHToV7aeqmRmH7H+zOUS5ZQ1pOz15cjuV66JNxe+Ml3lqORZmSp6Y6MrAh+3xmSSxjQ5rzqVZK'
        b'ZwKFmNJCZx0RuAn37KLSIDGFlRv//432hkPau6nHvGpPx7Oyy2znqUl8lFpg30OMFoE/sBQfmKp6gCjXgH0NuH7P'
        b'qbEgWIBhqikmIywDGrswbGur5V/6oDzqY5h9NuDGWwGD46fj7PntEfDW9K3FwFzUIkoBpEbT6JOyp8YUKhfGCkCW'
        b'Qds6bEII7qYnh+xZqpWO2DWBLDXBeVfY2X9Pg9M+DV56cW8lDp7crOzNHOjtfg6Gh/jYyvBDbFYbrW29W0V6cZH6'
        b'9+ChPSO4DgA0EbDt0CPlqQxSLUJzN8DRu838BZqZ926m15eFbk/OwBNePH9tPROVr44SIY0RQ62oMZyHn7YyrO2O'
        b'SnwFWQYeyqMfVlAjoyKgZZYcWERI5NmVDfyXYOiDGquPYugZFnDHagJpUsQYbKAuY6ud5hFgEsi1Z1uYRD8HkE/B'
        b'ElZ2hRjkzlioxtFkjO4H0s0Ke/jrNNIewEu3nbE3hS/l7LRcbd30xmdfWV01PCMK4sbMFJn1nvPbeWY0gCbqSMHC'
        b'cKr/evZggiNS3JzttR35mFeOPDRfnaeoDyqvPqLh2xPYm8h0cfmsvB6gZBk9B+TnG9w12xC90Qg86AvWHGSLSt2j'
        b'xGbNjHgtJMbditboixNfA8Va0qpiuz6osfrInvuqoceUrz5fSEDpeML+4tkGUgmLFzhrpbFtFSS+gA73EmlUoA79'
        b'nEkmHLnUIZ43QQImCdMDB2jOwS64sVpgrg+qqr6T5Tfbe1PIADh8dvpsc/F/p2CSBIfXPhPUHkm3NusTh9VbcZwi'
        b'02wSzrAuIWizYXHtxAnuOU1bUizCKoe1NqO/SDv10aHj1cXZEwLhO+o6lxd1qYgCML4eG+MSu1dGj+yRkDo98Puk'
        b'el8ooyZe11nBJlYLkByFgySBOoJzPWwmmVBRyvQHwnE4gG0bD92UhlIa6VUjc0Q4XpraFMYggQZjKDuOH5onUmnh'
        b'MMzJIUmjqGaxPuMRi+ToADTZmrl2c/Lf0sgvCpreYYeulvUulVKg/3bYgAjUMnKl2dz0AwvNUIEsQBO0D87P68uI'
        b'Y5eFpR3YtC7UCOy0Wtmma0Kmb7mQ7cXlotX1/FPEnvPlVPEVYrM0jCkFSG1SfOdu7GyBilC8OY9eOBWvhqU0jOIF'
        b'AMDd8nCrNo8YzIrFKQ9YlQ9o1aZtUczS79+m9EZNVcA5w0IoJE2fq+N4uEpUF0OFA44cLDYjtovlpRgQBQKQPHBF'
        b'tKYv+thWHX+it8sFwwDREB5ZfZ1dNo6D7vq0bvDwPoJfg7yMsEgv9KYdMdVb8DUHvtYiqOl8wDB7WMM2K5YWZZbO'
        b'4aMUK2FRhjQvuYaEnM+Jf6VNg1+dCYpcQIkiXgFmSjbY4iaPVh8wTA5pWLi7Yt244DwcLDUtTZC6Uwc/dh5sURLi'
        b'kq9A475PpPHkp2dDSncUve2cSakPGOYOa9idFbM+WGd02lDtBDFkj5tVUEmgEE+1Pxc7/AwBkXMc2OUedZmn6rpD'
        b'pCzrSdv7vSS2LRz5JCz9wceV0ylubywZ650k/9Wj3G1iiz7LURDmERhDgUESG79ivVXL8U0D4KNVHxHsER0t0ViZ'
        b'fnVSO28sve6vO/OVGnNkIXqWhdvUBLKLwBDCkGgr+9hKaT3gi2WpHZ0ZFLh1g2jSaqaoZgSaTL6vX0v4sD9b7qnG'
        b'dlydFnQG8PNcm1BAuTEE2kGxs8JW/8ipd2VMDvZz4A3LhMZJTc0iAJIrDPcP06K4cn5/RzDE21puHjlsDxMJ973c'
        b'bUi7bE9PP7kl6G6pM2kJWcYDtKuaUIDzOCobIBGwHSw8sRNomUI7Z2RJHqel+xwnZdNz8i6sn5T69Cv7pNTnw3HU'
        b'pbThitXz/ZaxzRdnNwYnYF2hhs0wxnL4C4V4I+i41+wmRRBiyT21UNlk5CzLpCwpi4DLU88bho++Pg9G1RyWoG7N'
        b'XUx/iKUiEePRYZFGa9jLMlqZGfyTohsBGNIhQBhjMgX+XRuzV7yLBGAJIDlNibbEFQj539F8ezDwf39xszeGUqnA'
        b'GNMCdBhWi8bqvQlIEa3NLD17m0fPUaxFiE2evR88LUaETeQFtYwV+P9XaKUcwMrtnygUdHZ9PH7qxekn5YzPcyvI'
        b'ZGkpB9ZQzTP7SSTGzkxsZqotAZVEViiWnpOKFsGKI7/HnIepPfYB+LLW7MLwpHtALNse/UVt5fWz7rv+yiKWrx4F'
        b'JAZgMLy6ZH1iCAIo6Qi8E/8RMLk+S8yFY+GEgoDeu9jA4oe2AdbQViVLdaPz+a2Ly29ePS/tzuCy8cmz71Qpmz7I'
        b'D25+1gkvLC771deefwoMxh6GIzEsKwUmjmXg+TRlimF3JBawHi/YoQLSGuYYOdeBrWly8S7xADxXwLLVfjjdKHw+'
        b'pimsEju5156oSzeSy5SPzL2yWhnpz44kLncFQBB4GpicVhklG8MhQqN2ioODrdaarfUP1DTr4x+WbEw4vi2B2zrX'
        b'LQHghFDkgRQNdb8QGyt4dWiUwG0thApI5PETgcKQMqqzpbD8csCNwVQn4khfoWw/S5vi3m3aVvKFZBkFPRWXI1Xl'
        b'qu8eAcNHm7pvmdKGMI7dsxb+pd3kCWDZLYAkQmDV6NYUopYNd5BDkNseAV1aqJyZgIosEfOA7lGQn2ymqoURnuVI'
        b'zYbqF85VoBh4UCmuG2snh3GMVc1f1YMcf2xLxYzE5TzHTiyNJfESFZ6UNzXs9zLBVvzWpeArMogbLcmIDcg5t4md'
        b'OJBG1tQrSDfDQU8+4l0ebQCesZ9SqhFIMajH04a5gAxFrECcYwtpS2aykm/CyFxTnmxhSdXbhwyzhzXsDo/OCNM9'
        b'DDYYNROb5xDkXBos9S14YGSjABMgmfCnnHvQ1F0B+YxwyAFEtSpqpzsVQ4kqnl8wT55+e3QgwHsKkvzrBUluq22M'
        b'cKxIifiTcXbBBgn8MmDDL4Us3rIjjDKasdbpAXRDDSzboRTqmuY3G2vC3mpgN6Mr7rSOXj0xxz/y63/sN/3kT2z1'
        b'QhY4b3zIbSSrvU2be4+seojDzbrcU+Ar3sYSwGf7HPhRhU3AiOkpL4VMKyc2v0Tz/AHNs8dEtt/eTK0+YSafZxfb'
        b'qxf8O/bsqSCZchqda2BrABSAuY7H111magV/N2ebmgKvyAeMxZKn6EBhrI2mrE7X0J1aorc+dVOEcf/2wxf72L6O'
        b'J/2iLUgDKXaQV+ONbUYSWvP84vT8+oTinBt5B2Fdme8tWrhPSAA7qk1AN6wbVD7w6kwarbWqPfGEu0vrdXjTsaSF'
        b'syd0TZRqMS3s1bTA37N9+2Rp21sMXGzLbG2bJSCgW6HGPvJXAJ104FxSpaRq5jTswHGFN2mgmcZOGfgrzo7InB3z'
        b'QK3VTrXRxzPtWTk9Pz3/6KRelvP2FDhxvNjcJ3E+KpYuFd7ophBSKEmTjRloENyT+uJYsFp8VWSCMmINrLPnIBzT'
        b'TAIQHHGta5tnVjvlSd/GvmUHjm+P9uL65YD4k+eXF8+ebxZMhWVFXgA1IhJVbwaYluMPPDboSJzWGztQbq0tKUuh'
        b'Yxhx5mYFuYDaFYhKK2MRKcuhIR8wqMTjn/jx3/CNb2xL/3ggp2zuzSRTBbQj60TKYhVVzNQ6M7w04qy7mgxyl+SJ'
        b'dwBI1Y0Zzc2xpkv0O2HcRuV0Bw15JXznQHVkQwU9wVaYqQjceZpRAgWdJhUmoswJFFSHIoy1HDhgGAuSJuLf9KG4'
        b'9b2zU8QUr9mblxuHWfXqvu0T33SPlRid+HpLy2lmU1pC1nf4TcXGQCSTVrJl6z/SahA8fJq2cUAQZ7hYEKbAAlKV'
        b'1Y5xjSsIwcvNbS8tuMea8LYhWqllzntjqiSPyQ4AJJQmrrlQLC96wMyb4eE1u62pdWWA+2RpT+OAyJBkzSt4h7BT'
        b'DfUxLbt8cf56iLYWOPkIu35gu7fSYGVxA7iuxayuGs4+5RQ7+oF0uMpEDHCmIN16FjaPAEKSHlg1v0/bdmafIMsB'
        b'SjFepJYEnMdBfL4Aa/dRxgAmZXdAnX2IUuqK8t2jeMQyy0iGpLQmqEb8t1ND9dEs2518EkVvOa94eiSixNmtLbO6'
        b'3ubaOaIKCKl2hrgGo6znLKSeWXMFU0012cxu1znhTg3VtzDvwdzDuXHU0vB9gp9P4Hae32H98PRgGAUgSLIauJp3'
        b'DvHDUjeuUu8gWl884DxMrev9DhsB1R3h+Wvlo+WE6Oqbp2dnV5tIdzzmPG2n5Eb8V0/uZJOlUjMjF5qp2VlAbrxe'
        b'JMKWQmXjN8eGaqf+KUhrZMsGkjn25OApex5AczBj9+P+NT5uetwA/kUsvBxzXPJxXoMYPPxaAsaixhAnNf38cIUT'
        b'7JDFYg/IyaWGWC1HzYNkpOmn6dTyCpW6SQiNzSUb8GLSuhrDThFV4oPbq50vlrBeD4Svneid3MKNLUbwgV27MwLo'
        b'OEFqdqV1nWDS2rLn6N8KmDB5fJ7gbREQFQlsACxZ6oIIS1XHOmRN5lf2pVey313tAUm+K9sDkvsuaw9I/ruiPSDp'
        b'd0l7QAq/wtsDfpFGxsNd1t7cbVJLfhOVbstSJTCvAoDGCAIOiIE4bDsCLGCemSx/Dwq81N1ImTVMAe6Y6bUeMZn9'
        b'7tjMya+XKm2EWR/nCpBzfzfjY09Oz5+/uD55CsMu5tyMgM7Cu1kW7A8pOQqcramalhsYIBYVAHxhErwxY0E4J07W'
        b'WKyXXmYoBdkISYbWbF7gK7b8Cz/wQ9/7Pfy/RWVhI836eEZxmPFOwxYZFsrJgMpnldlAZ52otN6KMz5gN+ngoRzP'
        b'IAF5Dbg+TyVboSpmi0IZ/R1m/ei/DWuyeVxrzk7rZhA1hQcCCxq69RkPCqgGAl58iIo4X/jkA1ZkgJykpfDozbVQ'
        b'R1FAAvwb2Y4puO/B/zgf3D7ug29mr5586/T6HOBzs6tk2VVNfIsLb0DQyw4RMMAJKG8uhkPaCuI45SGIuKvVMjXg'
        b'E8baCg5YNiVw91nxH9IKeVwrtnOITzZziLdmxMyzejz8nOxNB55yrvtWgnM8UQxwGPBTC2NiT5x4lAoFsTiEikMX'
        b'GhJVcDvN+BM0wz2uGTeTsE9uwjPtsFY4aNX7Yb0z3RfDg0PWzVspETjCzxx8DKYiDweL1MR5M94msGYLh5gOuXf4'
        b'XXb8+7TDP64dXIZXbcisJj5qjUKvVQAEpQfe2I/GfiyeVoNpt443rhTVg2Mn5/Igv2iz1SDeATauOobuJejetUKc'
        b'sH4EvpxC945qFUiB+C3CqrhJyXI4PSCAVESAQd1XJEaXOAya08uUVwu7rWDq2GijProVl+NbBRRzM3p8scWDzbHO'
        b'FU9e+dZBO+asUlPvU8aEd3ckEq2OU1XiggoighZy5fQBGF4TXH/GXbb8MdoSH9eWrRGUCth6uPe8Ck7qc/aN05Z8'
        b'5iBeu+h0m0ytGqyJFE5gUxdNCZ61aJQHdTFRuCzGtsuEP0gTHj2TXz/dPLsaVgEWAOM4fKZMZBE7rQVjKo4KFFEq'
        b'DJLO0SecSTp6zR0Mn9HZZVGOlCq7np3SVXl/CXsZZL/dTU9Kf3Z6dUXJhmVfLaeBXXwsKmAAM3mKj3tlJZid3mKL'
        b'ZwEo0dk4cwhLZcAdcphlDo3wom42amM7glUwj5y6l5HvJ9tB70uw4oEEx9bPnCrL94CWOpg7wK6OwpmR4G1tUOY+'
        b't8lwbAdCQ2OedzVFjvuQOXfZ8Cdpg92fDSebH7p180j+DXyLkIt91a01LNar4J8VaBHAw6otUtgviWcvDpG2t2kI'
        b'FeEplCwduawgqWAeMZX/aLn86OJr1xfPzkCkOfgtOE5UHLxRr5ItcjNQOjjK7K3PzoK3xEQuUkwq+IRO1n8M6jUD'
        b'JradyfsP8MHdYz84+yLAGANPd486ntLMAEAIj+6iU+eIDDlVsAyZ/Yy2cBwjyw0VFnqslfXdUlun6858/Uf56Gv5'
        b'mr9/0i5Zk3v79W2/9LJXqGrO0uQT/BX4/Rb5nWwEpjcOIAweRyDxeMOIgQZc17FMNenm3M37mRCUOKcJJC0BjwDw'
        b'VcMmWpEGiN5rTzst+CO0QB/PgudPP72i7OHJOSczLvRhk6gVNM7kYCWxW8KCz03XAQqxMpwonisP3howUjKZSkVG'
        b'sJdsraYDv0fdmaj/LRoQHs8AHn+35cHTosHLWTvFTpAG5OE8Zg1zdoMNZWRguyz3RYDdjZeWyXTOa+Twg8qeB4dI'
        b'tQIwgonf4XPjWa/G+dWLq+XJl0O/k/a0IBlsqcPVWKL/E+blDZXbhJ5sDLuUWzeDbbqU/A9jZM97Vy+C3xXjGoia'
        b'gthX0FBv/ELeMkvuJNC1d0fRxYPTO9nyvM7rBU2cvJyfsSFxurTQt64BW8WyGs7yOgEuYLAtRqiKhbAdIVM5R1US'
        b'XLpwolkL8F8eIBSX+ur2yd/ho19/+nxcbTcPdsuTy7Pndxdj47fRUVG8OY7AQASvYkCZG4eSlspO4ogMBdeeAbG0'
        b'NDNb6cojVrdM8UIAAvxQu4Lpgn3k5PvJ5XyFe1JNmgwakX1mNjcajs0abBSegEexZqPF+YoPWOq9c/wWB7ePoZ7X'
        b'xV1n314D32fCz9AE+/i46IyO8LotgfRztDwpnB0jKAysKi34wmnnOSx9fbFqConjZ4C4AV85fRK4ujL04i/Tmi9b'
        b'eXxLNsPd3rCFQymOgHNqy4nDHkKv7Nr2UXUumsycm80EEKKrNcFpEHMDcK2vVR0wXXMbxYH7bPk9tMW9Qzztm8MX'
        b'ykInXkV3TpEvYDieitdBOcobG2UOzvUq1QlvraI31LIMs3kP3sYeZ3YG7+Znv5eP+Y6Zd55+tN3nga5akDQpf4jH'
        b'ac1y+hP2urKipsU0cwbuaaxsKoBiUZCksGt6Ao/pnBGzO0P9Pj7qu6TY10OLIjuyiTBn/FjgD6tgI4nimgMuSgnK'
        b'4Xx0NoXpYgjF24ANEUouYWZ8NXOko+yM5v8qn/ddMuqN0tqGF3KwDFwPyBERI+nSytgNhRdTaiapMaF0ZlOWYlRe'
        b'3OeqvnbOdSnVNmzsshFivO9h/xAfNj4+AkNI3zx+yosIteckPUc5Ex6CG4eEP0rHLgbjAC8HKHCO44Qp+mEGcMzA'
        b'l/GXmdx256v+3Xz6BxPnJ+O8X1w+uQSiver1rpNlR1m5ox6ouQfQxH7RqBa5u9sRMggTB4uUzBoQK340VlvaQSlZ'
        b'KZyDaBbqtBoL8ls93knl9cjoGwcD+limjYIUSGe1xnI6A59CnOIQpdhqtD2GFrsqgEdKVXlvNRC2bOiTfV1551v8'
        b'l/GYYt7iMbfPeMJO6m3Q4iEGnRsra4EmgsupW/bpRlZChZr9RtpkcJJ7nWwUCZpqA2AKjWW5xqTV57Rv9zpPrwf2'
        b'8MXlZlPaJWAl58BPRgeG5lxr40HJug2J933kaiEA59WcgCV67K7OzDmoSAVgk/hHefU55e2e8+L5UmK5hUA+8sIO'
        b'e7R0AHw2k2olRxyKN2fnIPMC1skTXzCZRbDsHRhASHQelpEMv5u7MFCJe7vnfH56fv6y4mj7VvlSXbQ+R/BaQSqa'
        b'UWYzsVcfHKtWJki5QxilhH5o2QEjdIZWjXA39tO7nSdY/wof1r/Fw7aLsxfPzk82U7M3WCZSJhobEDuPQlmsYmsB'
        b'LxDuZQsne80JZ0ci67FFwHpxVLbFWy0k6bEjdo2dFxv/Eh9U3+JB5zw9eXF9erYhT9FFejxw4jD86ciXVPNKDnEz'
        b'OI7FjNgTQOatsMikzYb/zYR8C8AvkjololauLYKEx0e/i/Lpyc0IYb5qR3EgTukmxq2m+W49ULvPvgMcN7btULca'
        b'CH4YHZVnCJkz1nzl1D781zrduSf+NM2Ie0TAb9pDSis1s9QswRnT9C4i0mnuk+2iGetUWOuBLyZqkyHypVJKX6bP'
        b'eM6HLjtPoP5jmpP2CoPfMIgD9Y4Q/wCBmgEIV+W5TZ0GeL63aiigg3VBpm4OYE3GoBQQEUdRgD3AYQlul0Gf0aAH'
        b'89/dg7LlqAObnbMCKWlsKYVgA7yzUiV7SDIIWQmYUQaLLpuPvOXnoYh0A4Cu2Ea7Hud34XGc+WKPszn+gt8pL0kc'
        b'F5RxymTEqOj6UNYzzeQAamwf0cSsZKBANo5iJ6wpbyERske/0w3/eT7QWkL72ken19TAuGIF3lbZg3ibWuMcBxko'
        b'EJaCQ9rC28ictshBToXqgOAJVoGnuos5eL47oEODLFEnWyzWuJiTxw8NS6XqG9svspTiCHxrIBljrQEKRJGzGghZ'
        b'swBjyZiJrdacVTedsTkP3x0yzYyxAwAP5JWdofjP0xa3x/hwv1GBLXI2YfVbjK2AYyr2QoVbYdcIhVeUFVSOmlrU'
        b'IE9jDkImX8DpIsdvj74z5v0ntMnvNUjcbxXnXR25AY9PWRMoEQAwspOrHVkojxjjMpAUoXwCe8JRHBu0KelNX9aJ'
        b'X/pOaPeztEoff9ddPz3d3rK9Ygv2FcUVS54z40mFQzyUym5xUPHHp+rYE1KU5W5YvImYHmvncnlwk2RTRNLaZczP'
        b'0Ziwx223w6rAo3qLdegV3qPNIjsBTw/O/4FvpTTgY6TmrF903JMIZGTpQOjCIYra5s5Y9fM0Ku513+0wi1O7jmDb'
        b'YDE+TOmIZsEDT8Qy50A0s5zNCiMsde/BL5ElgINs5PifkYLhxdcahHDp8TfepnjrTWOsiYpVwuokIAhOuVlE7Klf'
        b'2jLYvEzno8X2mdmlNDkMUkySyJqgysrvHtknvBod8h633i67AlU8sSrgx4BEwEV1wBxwe+MtBQZLAbcnmLB5FoSN'
        b'5K2EZvC3CdmJ6AO5bOUGOHiz1723yy5OIjtaug5YJ1QaaCCndAZQnCSAALbVqZkHM1Jmb4tCCoCgIFCAZeBPvpmd'
        b'+OhP0S67h813Ou/dezZyxF6YcJWMgNeHNQ0UZxk6EKaWOisBTArsrvCWEzwKLzd6A7zI2g2IXHFrYc/LPvfe/WYt'
        b'YY9zo0CSI/ibYFfl0eAqQK/wLy9pqqN8ZcvqvJbosesS+6/wOVOc5qprYc+7/W69+83ahr0usaoTTspGROiB4yvF'
        b'gNvFiaQbqFGBiIfP9czTi2qRlHnfoBXWtdWd5x9/512dfvvenSfL/cF0iMwNKyRgnxnxASzP1SF4ZlgBiICEZZTa'
        b'gFLcojzEsrzuCmB2BVpa3Xm6x523w6xl5+GRPduQEbS1pMRclJP1gOKxu+RZ2MkCPc8G5TErBcLmjN7YxiPHOld3'
        b'Xtjrztth1rLzJnV1kFyXfg0LsldAMFyKIBN4aLXUB5zgt4LAjUUDU0MsCVStBgZcwa+LWfvk7FfjE04xuodrGGzB'
        b'6o2xrKwwy4kfm8YA7qZLNcyBpIv9CYSusB2o3XSKHCDs88TFeSD0ndc+f4Fm7Ze77zSMCuZHVTs2FekdAmEucBve'
        b'ws2MzEsYKzWUCtsocMHTF2B2P33HatpWQkppDVH4fSKKzdCc+5ZrOaQbtikxBC+sKY5k+yIo512gJnpVeB7VDgAA'
        b'pQwgCuRkUBHDFs2w+8ibiEL3iyh22cUxoEecA5aKMx0UsWKBOBcbgV0RwGebPUzPIiZsze5N4GQkD2gVNSyoPdm0'
        b'Fgl1n3fCy8yz+yIh+ZRWUzmQgAIaydnWJAPjhtoZBOFWy9DbbAMiCBBGVywT+zc5ggv0OKyFDN3v9fAOszhb9yg5'
        b'RLQK1xmlAouztK/XVvskrGi8Z61uYP16laU9PYO+F6uupNaajvW12ufpxPXYvVYmuyBGvDO1s3+08NgI5IPTxGwL'
        b'HQAq8dY2NJ8yon4tsKVazrqMqbmxulb7PZ7YYdayVop0JdWTuxdswCyZN3qmW3xpgtjbYZ1pALF+mC5qluoLGIkA'
        b'WG2yunY0q/tEGDcdQydLH94mAFIpNlaqp6UYQM8dEN7g6au4pQ47iAeD9+CIdXSAJcb7AmSFKKF0uQCAv8ugP0eD'
        b'9osu7jFpuW3l9IZRAf6s4SDLLr1UBGsS4NZ8KN7xCIalqJ41BNnM2jM8Dp9ThMCddQJ/hibFQ6xRzVK2S8QRqm0G'
        b'PGTU4VzJdrgpnEhuYUnFbizSqOZSJ+ULW/IRWCMhewF5NIpL7wx7f5b2pMMs0a1FnJZ3BM5qSkZ4SCHXAZBXTBVf'
        b'fcNKcH6DBUIqOnRgCaNpzc2YfengiKbBUru6QvkQKzRA11+uEIFBqT2FySZlox7PrDHFVgsiRacIT+GCYL14g1bg'
        b'PGBHFHmIjgXTaysUzGFW6NaiZYVMRgbFA5eI4EyhFK8I6+xqEPB60HwvlIDqLMh1I4gu4xNDBOOnQnlZW6FgD7FC'
        b'CFvp5QqpDWNQZZXNGVmBJMACVayros3kWADqeuSM3iBDXa85A78XmA2fqtGurpAcZoVuLVpWqMUE4ICoFU1JFESa'
        b'3jUHStX7pFCPzQ1UXlsGIQ5451YtGCGQ7liKSmV1hdwhVgiQQF6uEIxTP2MhS2dooCJcnH2OzlKOMEg5JCBCgF0M'
        b'CulOagJkH9kbDdq7ukL+MCt0a9GyQnNaD8DGe2jVmnLNk726ooDdJjqOpqSKoWSH3ztg19zMEMBZxveS4uoKHQYr'
        b'1OperlDUGRaxMOSd1INU0IXJng/YFyzDH/h7cS3NyAJTFhJhYSYspcaurvvQoaDCjUWbPJSAYiJ4qs0+cE53DaEa'
        b'oHAAbjuzAX21oPNuWmzFILyAFdUEwi7Ss6yv0EGQQsR6vFyhgJDWLNw7y+jMlQHwWjQgLpgakYuymuFN9KCyPCCz'
        b'gKo1ciCK5Dx3X2wvK3QgpHBr0bJCtsTo1LfK6Tmc7koiQdEymAeGNJeTyhKTnc0DxibDtoVsxU1A1NZXy8pDfpsx'
        b'DJsBDMtSLK3424EMn1gK5HpVi/fNLnNso4yAO2IJ5DOIXtGz4m9Q1xcYBy98RPyyaNX4MAq8yE4kJbU5LecKmyaT'
        b'V5/738FzR/MOz7155a8/uV0GRURqBPHuO1sgY0SuVHoEx+kR3wJobCKLiKPmLUfomOGKB7bpaZkb3Xt1bnu/d8+T'
        b'X3/+2VeWAYTj6vNf+09+/ms/+8rZxUdsRPmcf/j+5YOf/8Z/7O/47Mts+/v8s1+3a6t9UQG4zz/7gaunBYuB7/Wu'
        b'InCfv/jsBy/aRm63jc9/49//pc9+1U0jDZ//4+/7vu/5Hjz8x1/Gfz+7OLDU3ecf/wB/7CNY+YK2fC++2a1Nv+PL'
        b'G5u+/h0vxxuqdTeP+a6qdZ+/+PhX8dH+gS99/Kvve9SfOrge3+NZ9uYCvPjSxqof+eILsENa7+Yp31Vb7+b9/4Ov'
        b'vf+bJ31+aM3AxzPszdf/d/7NG6N+7Dt7/Tv0/2499R31/26W4B96bQlunvb6fYgbPp5xby7DX/3bNobFXcvwgE7h'
        b'zbO9q07hzYv/da+9+Jvnm4eRXXw8c/iqX81tf9+X7k/NP/3DN6nZrfnCDqXZu9n4XcVmkY1fzb7HXIKf/uGPv7a6'
        b'BI8rnfv5x2azBO9szLLbf5gmfP1LH/+92121mPR7vn9j0m95F+3LdWXIGzPeVRkSjiH3Pfo/+0HpYD6etS8X7Udf'
        b'W7Tf+rduLP+HvxPEtCZvefPQ7ypv+foS3Tzo1XuQ7Hw8m16QWPBJETGXePVrbrcA8XEbCyn9/B/fkI6/oQNAnJ4v'
        b'pnz+2Te+YxJ1/yTfRRju889+8OXmu/l5N8Ez/Nab4PnTH4LE5uefffny4mx8/tmvvvlrczdAv6vw5ueLd/wj3GTf'
        b'+u0f/2au9D/zASltfv7xP8pHurXdfv7xTy678V2VN+/Y/Qt/z8bu3/5hSG2+brLcmPyu2pt3TP7aD25M/ifep9jm'
        b'64a6G0PfVX3zjqH/x9+9MfSfer9ym6+b6m+38Tvqb94x9Z/+Gzem/rb3J7j5upl6u6LvqMB5x8xf/Ns3Zv6uAw39'
        b'3PY3Hy8HYlt3vfHdN1f412w+9uoHbh1Y3JiZ/TgUwWZxjLFIzKMOwEy8BZIrzybEAZPdFM3e9iizlGgKmPKdt/B3'
        b'bR34dxzoLSznwdef3oSri8tn5d6o9Te9+lc3lpeefIMbZxuiNxpN5SiyQIkANsaNTdE7sEJ2sL1b0Rp9cVQtpQbo'
        b'3fX/F/+WjeX/3AE9+lWj7p9/un0BP7T97MvPAHmdUkfm5l2w772xoqsgSyk4dok0M7BHc84kk4Oj6xA/csYbQoJX'
        b'sJJq0hzV2jruvIvf/OP7DuOvqNASXJ6dPlsOjF9Kz94Yfvcvt5/ffObG8GVeFnJ2bZaXOd5ZcWzxbzYJpcdKCNoA'
        b'2hkREvXtwNZsSbEIh+Q6uWP4k69sDP+px58JPk/v5C1YfFHHzXnAeGnt0ca04/s+/dreHz16r1jo6a2z04DVh8IZ'
        b'abGrFbiA2u5NFGoJAc8E50Bwlr54y46+fsfs06+sh/hHM3vj5kBhyx3QF7YfqPT5G/bHAWxmFXYiYwdj2J2Jx8sT'
        b'ubtQKmlSxWIUZTWJzzCmSI7OW6w9ckC9Y//v3cb+Tw8zAfiWNLyx2Y82fzf68Sf++P4P35ivrtrBdjJQJWRtwwny'
        b'0wOZzVCBcuwieErJme5VOGlNRrZw/AguG4Hk7pj/8fduzD870GjCu4H9h+5avP3kZrrpraFs7bFJ8bO7sbMFG6Ug'
        b'uLXoZfoEwu6i///Ze7cgS7PrTKh0l9q62JJsy7Y8PqqxXdWmsmrfL223cFtq2eqjVstSh8eDRk72tSq7szLTeenu'
        b'skceD75gB/kyREEEERNAGAiDgRnDgANjwE9EcHtiJuCFh5kHYuaBIIIgeIAICL7vP+dknso6eelTeSpHKqfUVVnn'
        b'uvdea6/1fWuvvRYvDCcHjF7ZBqrkYuAKoBkQzNxE/68pIn/zaU10c/vudJ4/sGCew7NTZc6lKx4MiAbwKQr8FDy6'
        b'iWzQWllwNHfvMjZ2i05Ck6F6MrsEjweX7tS8C78/leb2U2s0aecF+oM7aW+P07SLxQl8CRON2fioo9CsIFS71K00'
        b'pb1uDfwLxD925vfbCrtuJDinBucsPjTX52Y6/snJTLee4kyPJfrpxyY6J89UWYsL0uIpKctUqJhdAE5hXaVUusCf'
        b'modyyRU4abCMHmWCfJPu0th0pfJ0C+XpFsuzCu20wfYMxaaigDZCVTAwBnRYBZhEk0E0TO1AHrwOkBWtJG+w1ZBC'
        b't1cqT7dInm6BPKVxUgvbpcuygwSDIzppQZuBoowwvABfM7vhWt7Mt76DdgzlynTlVaN5ef6Dj64MY0zhEWbAajlr'
        b'mQUGZuGQYyfzIxtbte0AX3JRTrx2Nt9YZQiu8qIyHQvsrkuYpPKFj0gjrWQplQb8VLLxcC8wvpIIM3XTi57MdwLd'
        b'N7buweHvk41pUrdhFV6danU+q5/gvbQLgQzrYHVZyzKJNS+bXkvWqTWAvsg6lT1Hd9xLcLoGkzoGU0We7UmJDZcD'
        b'KD/si3NNeZl5Uz6lUh0eTBoysz3WCD8iYJRKjryC5IGUg6mnzslP5/Sb0zndW9GcWKVhbe/g/v10xIsnMyvOdmcF'
        b'SyApA/JbaFEl5hc6+G8OniefqbFeeNRgS0N9r+6gQUkBG5e5mX2OV4DnOMrQ2H4tb9cHawzqbrwF/DVM8vV3HTid'
        b'tDqaZpwcf8H0Y6cT4kK+8g9//j3XZlMLcHAGnIXJKQnQlSUjgXsD+//2wNIdDVPgdTcYncaKnib6rmNIMRjtHh60'
        b'w09QGba31mfB7OHA6Gunxr1X3MxpOsf1D753Nscnbeo0PYp4hR/29SeLZ2/VwT7urfXd1n5tJpSv4JMn0ezJ2P/o'
        b'5yCfYQF/+ynFCYY28XsNS1uP6DI1c95TfWr6/PDak2TBNCEk75/wbqRvtRkbdS+R9a7Y1s3l3uHIpAsd7Ai8UIkI'
        b'TxY11hpMa85+f/6Tk6k/fIqBggUzOzNa8Jfm33B6yAC6Fa2XAhzBt5J6FIFlfId253q4AidigwXUpfWasUQBgBsA'
        b'u4vkZfLz0Ppf/tvvIkr45BRqsZ5OLNjjHOrEq48iJhYourHAvdbGZ2OEg9MrpUdVo5Gx1cijJ82a7CyQzxMA+IpA'
        b'bpVTm6dQv/HH7yIefAkNeyYYbK396sHGW2mzHR1ATTHbYLQff82RJ1TwcCW2aPpwAzpiSkN/dVYWYBIB3EYMVtmk'
        b'oBkANT7GJnL1lUWt4zyaiT8wmfjBJWO2aUOifLCxWU8eZzy283/0zJcfhUnh/4BNWXyATYRhSoHPKpxIx18s8l57'
        b'8jGxxJbCw9AH7Uv1rMPfQL7mY+I//BOTWf/zl5QMy316fFq9Pjut3nkwneD3Tx45mt9EGWYTU6JnxziXTw2jtSHy'
        b'3nSFywSVNMqxYLoEdWotxtygzSImowPPQGIGXp2b2OaPTibWLnVi2Hr766fP7tMnZseX7x3xCx099mVkeV/gau+Z'
        b'SK5jtcBIvO3E/k1ZtRSFYC2glisvlSdfco5SGj03uZ0pPnvjcnV1OvrB1nLo0433yIY85TVHpBiOKHjhO01SzB5C'
        b'Ky1KQDPnMntRYURsOmpj1rygbAV7wTA40GGY6jwp/lsfPBtYX9Ik54nTY5ObI06gftoF7zFBkU01EmjGy1BNgUSt'
        b'xEyjK6zQWW0VsQOAV15GhIHN1kP0czMrU/H1ldPDedH98CL6+Kj0ejVadMDrBOkY0B+v4CpllDqxdAa8q8qRvcjB'
        b'kzKgHTZgSroKyTRwGJe5OYrXZvtv5XM8FuAPLprinAwtrCGkJ8lvlVU8YilWKBudcCw9L3RwvOyvdStBNV9APmLp'
        b'0NwGt2bblQYz/Lwwf+TECyeveVSaImjqYQjZA2I7azAF1wdMZD1rSTJRXpcgumkGeB0OkenwLHofspHtSgMafk6s'
        b'n1k42Tm5RriG6hprPRXhi2Fd15gKpm6KM2AbwgLzgN1jY8ZYnQ1VJ7B+7wACgQjnaOOn5/ODZrxnyhR//jRKsrNN'
        b'J73xTqsAuwsSjMyd2SfNEadXb39sJiiWOrHw0NoboeUAYALYvbM2dulJEJWH+64i+Jy7AZh32QWhdBC5uPmY6Qvy'
        b'bD44MPdZ7tLeRFze6TUHea/h++VadKmuSYxbY9z4AnvM3vvu9q+1rbWNLbDDgzKU3r0j1l76+Ze/+vo35s65Jgfe'
        b'TGicf+maC0eKaVwsLUhbS5exVs98HN80GDtPwPCIkT45qW3tDUNJLjQLPxIo59bmI22fnfLfv/m05ivXTs2hPHP2'
        b'XhzhAKvg4j0gjC6gycA8AO+aBxtV9cCGN753Xhh0TORomD2UInjN6irSi3nM+jc+M5n9wiz1R1IGh9zEhZq55C2N'
        b'syd7lL5havES+88FIDRriwLfkrqxTZexWoRWSmH7Rh59VFVqbkZUSD0pmbRt86Dnz993RrjxciZ7ak7+2bN1R/4z'
        b'ObgSqTQrn4vuwLw1eKjKKoUsWHmlMzufJ7u88yW7angKTCVFFv2YB+Z/4xNn0JHLm+2p6e9nz/hoK5uQmJYQnAtD'
        b'Lz/pI6AteHyGuYVUczLZwgGl5rODUCsLO4kiAttfej+P9+5+ejLju5c143Pzzs+cYzjasEYBF3nPzpECkN0D6RjD'
        b'a/AsqesrQHzOJVgApsqODT0WqeCD7HC9VPR58/zb3zeZ4+7TMld+7RvjL3/lKxeY7lEWnWWTyUhimTJvN3Q4T6YS'
        b'+gga6QWPJVmnMAcBL6oibwEBEVYhWtEddnpuuv+2mkz3c6eRsJO+4zPTSNGvtfrI6I5koYVzLFlZgEuTgp3ooiVX'
        b'rGldO/h+1TtQXG4WFrNEx8rCEBn783j21pi3J7/9U5PBLXUD4OzhHiVhPnlC/REwOe271BScPPkdw8my/NL3H6cl'
        b'POENvHPHrqdjv5QLea9N7829fknX184dvZmO/jJus702vXT2+iVd/jp38HY6+Mu6C/ba9MrW65d0bercCbjpBJa/'
        b'RfXa9LLT65d0/ejIsvx/Uyv/xSe56nLCbp+2DP6YA0G3u41aAkdjReGDSnDZ6zoUZB3aOQuwWZ8Fy4gFp2LjeUFs'
        b'wF+Y17zZ/sBk/Cu9qnPmrI7wxZNfbjlXkeJUkS7xjstr0yspr1/SvZD2xt+ZXnb5Lj3s+8byh32TeHnZ3N6bnCUN'
        b'eWgLT/tedb/ylM76LnRt5UQq+BHE+YsLK39xYeUvLqz8xYWVZ+bCyi9NVfc3n9KR8yk+4wiUPD9/7n7me47SllT1'
        b'HrwZkAcmW1bYYrar7ryewZ6wAFgtRGafOWzbyN1tYL6TaQYqH8x3r4dvh589Djisz53gr++k3b02XHKdXt889az/'
        b'8H07G1uTV06unV51bsLRTdjf+OP5SgBPnItw+H3YjrtpsjTrx9/x4b4B/gI38/CVa9fec+3wufYOprz+Znsw3M3l'
        b'leFBOx8ePj84n/VJ0sy0RUjib41JY5M7CpRC2nx4+L1c73WeuaxvH+zv8O7xx+6nfbhuvp4H14c/Pm30zI+YfHI9'
        b'8TGwjg8PP7LV3l4fpPDw8Ob21uaD9SkeqKRbb7UtZv2zpejW3ba3njY3t/HUQx63YmV38arJfKZf+sEJeuAvk+SA'
        b'w09Mflm/n3agCXf3HrbDH8EHv9EGAI+F2dreGvTrWBbOa54ZRNeJGaCi0YkAJfCsZy5YAjaG4OGPTIjO1uKYY2AB'
        b'n5pimbGHh5+a+wJIfrviix8e/j7lf7se3N/Zu/nrb77w1qhv747evPXWaGNrNORybOy3+3s3nx9t9NGbo8+9ODq5'
        b'xt++BY4wEd2Lr+8etFt7kyjm9u7eizcHMd4q25vbW8/fYrfqXSz9XtnYGF76/O1hHO3mwX4Pzz88/OjkE2d68gu/'
        b'e/ix6SNHy6DYqTVjW4YKAJGFSowwRcM6dtKm1i1ApWrYs8oXuORuwH2lGPpjgYx0Xq0+wN785f/3vdeuvdT32+5o'
        b'/14blXsbm3UEm9LS/RG9JLOyyhCKvTViEu5oEOjmA3bSVKNBqUfDMPn0EPkYUaVHkOatUW2c1ehgDys8mmSVjqa9'
        b'MqfP7fJtg6pMPnjE5bs1mugJln6/3W278tboEe09ehxPHOnn7EFxe/T16SdeeMeM8PDooto92t8e5TZ65RuvfXUE'
        b'ptmOv+8iW2r67ukqjtJo/+3tNSzy7oMRPTW1DaQT0ph82Nr0fJTC2ZpMaK3OHvzpUUvl3mjy7ntpb/Tot2Hv0dlN'
        b'tyAEcrTLHh089wJWPG2mXUgHBjttQQTb+MbdEZaB4sS/72/sUYx3JiLnd260vWHh8DlYXHa7He0d5L39jf2DSe/O'
        b'Y0Gc2CpUMKoE3z0R+/RxTOLenT3w+Ftzz/Hdo+M9OzoyCsPLb49ehxsaHbsiKNQONuKobmN8W9v7o4mIqd1QllHF'
        b'gMq+caNZkvLDw89MBTeT0fps1oMJ/vijkoDNOiEG2KxPTpZvnUsK2bfNOnnvuzHaFzev7eDwU2V7d3dqwyZxCuAu'
        b'+tEjE3sDXnOaxwwgAOM1G+7a8Vv31t6SMMZzAz56+4ensHCPv25s1cE4H354hp8w5+Pff//w49NXz8996vunjn3q'
        b'V4+d6keYr71Og8fPOvq2bz08/OTs++gXZzbwmpgarMN//NLmppZHuj6aOxgaHU8NmlypoJO45mCi1ibqtLEFGNHm'
        b'FWaaur6PMZhwe/SF6ZYbTv1Hu9t7NI5QsXZ/Z//B7dFLgxpNB4zfW+U2GM1dWMC+nokRKryzuVE2Zp92FEz6aagm'
        b'Dew2LNtGGWFjNtqg3dH2sNHgigAapt5RBWhDwwc8/OU//+S1a68D6bWcYGlv3sfgaHc5XMxkc7QJPPj8C8+NRl+i'
        b'Eb7+M3v7WMnPX78F47LVRlLfooH5mfvbFUv5+aOX3TnY272zuZHvTLIw9G2p71BsfGg95XJ758HsI9jSlZ9BXzCR'
        b'DT5lNHp7Y//eaK9t9tvbmAjGVduLN3bzjedHMEv9heE1w89vzH5+5fjnYuPY3MYM50ZidRxGwi8cPn+3gatDENuT'
        b'MXA0MFsYya1RPui90eTcGs08Pn7b3YVvHlwIP/D540EeDfNX3vXPc5zJV7f3v0Q39zK/4YXRN/E3pK2+NfrqNjXg'
        b'3sRFQtgTYwRG9sLoxrKVMjtEfpwRduO559bW1ibaZtRgO4eLqm+Zifc4Uly86rl3oUkXKhg2jNELhockqIs1AOYl'
        b'y7Xmuu4usk+JOx7/Zqvw18eVeubGOidmOvlHdXbEvf3gxbu314dkzPXJOqzvwBu0enPyqd+8sbfTyo1vffMG9efG'
        b't27RJTx/PJvLzByddf/GcsyP28vJTlk0ynl9HQ68Hp0Dx77Ogd8aXcG4rZuM+7FxDaPmgG4Pt6MfzLJkb07rGgG3'
        b'ceC76e3pbwMIZAXaF2/eg70g2oMv24Uve36y237j8Z9fWebnaS6QkmEi2EcXYSpS0srRi6OJtk7X6SYoJvjAUZmn'
        b'b96YK+9E3Tx+4sQtIDw5b5VO/sy98fhMBu9hJAEwDP5vRaqzIE/6aHlEnFudYfQTOgHv9yJB1805WtFu7m5DWebG'
        b'O9AuPDj4mO23r3ACP3O3Qft3dj9/KXN47OdMnX/K+iyGCc/P6VFtBk66y2DSMNOrVC5p1SlDTRt7bfRL3GeD0715'
        b'gz5iJpQHA6cYVWjk/o3nnzt+2Quj01723K8f1Tm4/gKc5uYeYAQe2iexIvjZw8O/PntgUkh6gofwuGG33IA/jt9y'
        b'9JSyUksdw/FTYDMn3q8s07DnXzMNk8xeIGMEv+fzE3taF31IlJaZRHMv2t/ef2QobGEyeQUR0/pxec3J0/C81wlL'
        b'mRVA1gKSBvvUhtPS3bdanZsv+6e5bw9fRWdXoGn7eIILMMAs/H79tKWGcK8f0fjpmwaHs77T0pvTda5TWMzJezYA'
        b'81ycWRh/eFR5rdz8o+uwuzzRmL1NeeHVbIyY5/rERvE5JZyIfI5Ae5Mdy/iocz4IPzw6Y5+T79dK8cVv7xK3767T'
        b'2VHL+aSU0QXPpaCWPJhTHmyL/QNO5/oXXnv1a195+fWX17/05dfXX3r99Zdf/drr61966ctfefmL17/9KIDDmmy8'
        b'NUxhit++NkDjR2Dc7cv8+dLFXvbci5fy8xxn/cJouHwzxRoTxra3frC1d7BDtDEo2vo0ELo+vYAzujm8Z96kfWPS'
        b'F/AXJmccX6ALhTm5/e4/+3lK4BJ+3hW+vvS7TKfC0ol7e/fLcoLosRLB7v7XaXj3bh6b1OdfGN09SLv19iTqMIul'
        b'3Jz+/fxzCz3gY27wuZeGj4feTw318TcMQZ3B4NfnLktSXweDl9KNZvHFkb0Nc7H33HOTXTm6OQtQvCifP3WDwnTg'
        b'Ffdo1WBZ/qnnW9Ph8kLGHL/Xj9OtCRsHmN97kTzz5o3JHV+2651Vu5pe7TA3bgGK7r9549YNyPudB/x7QuXx29p9'
        b'/HEApeAi49e6sTckD/CpvRvHQPfGMjrPD9nBH4Na/xQmdONbz1/9Yk64690ZStnojGMyAHR7Qv4YAn/hMexy88Z2'
        b'78MHlIkRmynRjVtbwKC3Hv+M558/gyb8ymX8PAKazhjg6EzlGGH3PDz8Hsh9p2m3fuLw768smyo0q1N47O9nwc4T'
        b'p3r//Qtzp3oquW6tdd1lb1Nj10ApU/NWm5CsEbEKHu2yUJNlbSOgi84yFS72FFqoDw8OP3V0b30dgtmq1NlhRp+Z'
        b'S0Mb4OL6rKkIn/3w0e2j+eK7H/+e2UL8/PLl2R+9jTQ32+rZdLTK7nxWbE0lBC8hZeu8yMGIJqDxrTVmMMvOmkat'
        b'95ysdo3dYgyrmR+d0k3LBn9oFgHn6fYfvv/atVfbp9/4d99/OSnLf/T+ya1TOVz6x4h16DpFNq50rpeo2ctSqKHs'
        b'Vi88j09e2+Rd9cr5GFhoLVNI82XiJ6P8pe+/rFHOotxnpHBP5nEJrV0eLSb/yESefiuXy5rVkCvyd98/SZAY5vTB'
        b'T03mtHz272RsxcjSXCvViVyDic6rLKpUmRkb1J2gmZFmvbKdVyZrD9gE7H2Il1YZHx688ffmx/V/fPJJx/Un7z87'
        b'6Xky7kvo/fCni8b91Ho+XNY0HleN/+39k6lcyqWCyTCFl7z1k1IxLfVgW5JJaQc345ysNcCiQL9Vt60606FHMpik'
        b'vFK9VVmqOKklv3eZQzxTYfRsCpfQqOhPF03hKhoUXdaMHtedv/7eyawu40rHZJQVdiWyy1uzocXoNfypyT60Loqu'
        b'QTgMh2Uhva3MpApChyLhcoWWlhcmT6rOwSWO8EzNMbMZXEKLpT9dNIMraK10WRN6XHFufmIyqcu6TjMZaW62q+x6'
        b's6ZTg7uXJUSRG7XdS6maM5mVskMMOpjMkmsRata0spiMPak8P3jJozxTgeyR/3/yBlF/umgWV9QY6rImhQ9849+Z'
        b'n9feByfz+utP4xh1ms97REjW3lLTLNDJ/IDrZY3ReOd0ZLl1AatVAKyZpV971tga2EMxlwZqkMB/4BWTzkDcOWGq'
        b'zGY8yu+bS9y4eTTdT009yG88xelOC0gwi7dtH0xrvwxa/AM8x7j/2CuOGEaUIVsfu+8uFc7Wyahj8QFMoqnWTPXY'
        b'hE00g1WIJvnGAg7sDZ6hJtN7p8O877z36Yt5sgnaGuxh2pmf9SNPrL3ZHgyB+NmsQQNl6j4bV6HKAGm+VPAsE0CJ'
        b's9UAOBng3SgfbKg+2ZhAvzpeDCbcas1zs/7x90xm/TtPU7nnDoDWNu7fn2DR2/vv7E+n/2OLX/HYOsBvK+eahzMQ'
        b'OcYuVcix1CqkssCDWAT+l81AKZuCYSgyW+Xxili7sHPr8GtT6f/WVa3DpGfq3twq/OVFz8/CMtOE9yPfqUP2wMKt'
        b'8k5h9LZp6Z3lvZ0Ivu174k1xCZ0v0TjFOgDdK1gJ2ZyDUsytxIOfm6zEt5/CSkx39bG1G8Kucxvhh46fKtgIQ4ba'
        b'pKPFkQ54XY0vrrDWmGqaVciwAliPIKSWLhptAytBM6k7Ai0YC7/sjdTOKWUi63ceJc59bhoYxhe9dVQP/hFj/PDw'
        b'M0d5XUepbtMEL0ZV5jLj3vjXMMDJmv75Jw+/Z+6AbhaCmSXQT170Y1MV/L2nsPB725tr08eADXYmx2PTK1LDFpvI'
        b'YLLECkyQXAr+RbLlduZ1zxa1Tl237IDSsMQF5DwIbwGqrffetaa0wjs7wzgTYX7u3G+dqOFzH5jzvv/qRyarsvuU'
        b'k5jm5t8DbG1y0QZn4WSLSyDESoSUQT1ZHb4rDZ3rWnVsuSyqSSKHUBWUUFQWS5nM//rc5z8CyYbqt8Nx6oIF2Hnf'
        b'ZAF++ykswFv+zukR4PklEYXXl3uyKtiUgcYFeFKKJXmoB1CuKgoMDzvSiciCG0kwZNmKiibzWtlsSX7i+ARmqgdT'
        b'k7B3wKzXWYHFx1flA+97Nzz30o4H5paAmGO4Hlgd/UmrKecUQXlVi2C0XXaWsxLQiCKAaRnAArlsCusACtndbAlu'
        b'PLYEs++bHEUdL0KbTPlfeCpTnkDEtZmxW5vYOMLRYw/1wzNz+firjvA49gX8DExCst3AGgvoR+AtWaxHtvA+0rKH'
        b'vGcNl8yUB7gq69nh3KYQwLMPDg4/NPn8Sfz9E7PUDt7HmYXsp2H4V76Q33NtFoj/+rJJmdPM69ndsvkKsPP9vl01'
        b'IhoqeoClg7tVJILdGEAP07ppLMNvlck9lA5Tgf8CrwI3m3z2jMl//GgqDEQP03vjJ6Dlr/zZ//Cea2/8JH47/OUn'
        b'q4J9lLJ83GjxqGDdzQ8M4Rpea/ayStgxaEnHCFuWysE91sAd3KuyvQljWaXAFWXAx60G4KrY2LwFf/ixQQ3Wp/cT'
        b'j2bx6r/0/0zm8LVT5zB/KWxQ1iMFPO7NMv3YtUku9yNjjwrKnU2V3ohgWfhfVMAAHZKx2YNvm2Rcwm+mgRlYa8Cb'
        b'YLRVy8BGoEpQrsMP3k/vbMD333z12s++cu3aB649rb/xx3uvDRfHZn/Mnnvv0d+zZ9577eTD+MdX/+DVz3/y1Rc/'
        b'8Ort505+Jp797w5f+UdvHL/5ex/9qvfMff3clx79f+6J98y9fe6R9z76jwXju/az+w8PP0xoxdPo6QWH/XQXin7z'
        b'8HunDOfO9NivPXz+5vja+D3j947fN37/+APjD44/NP7w+CPj58bfM/7o+GPjj48/Mf7e8feNPzn+1PjT4+8f/8D4'
        b'B8efGf/Q+IfHPzL+7P7Df/HhGx46Mf7s7zy8Of7R8V8a/9h4NP7c+Pr4L49/fPwT458c3xjfHD8//qnxPzO+NV4b'
        b'3x7fGYuxHKuxHpuxHbuxHwd+yuHHptv+ztdffumLfxUf9ll82GjuI/i2ozfcHMfxC+OfHv/M+MXx54f3f2h67fzs'
        b'dz76tpvjf3b8s+OXxj83/sL4i+OXh8/55ATw35mYiDt7LW1e/CNPft7N8ZfGPz/+hfGXx6+Mx+OvjF8df3X82vhr'
        b'/CLI6KOTnLDBBe1Ne4oOOVyTNT18jlnA07scr1z7zfG1w89sbvRWHpTNk/lkD1/5gz/7w2uHH5tlnk3s2vgXH7lw'
        b'8gNMndjY2X/svVSbwdV/BV87fPcbr9Ic/sGf+WtvfHXy2x9ee+M1CvoX3/ga/vr9N36RDz/6xheP3njtZ2++mze+'
        b'dPzGaz/zbt748twbv3yBN3Jj7LbNdnSo/IHEW3wPx589/MBkyZbeDJQn9tbgwPeAoR4e6pl3Hh33epq7oTW7pMZE'
        b'h1Ge3RYa5vZXOfrwxj+Hv7ipfuxoQ81vo2ELARH8NarKF3cO8ubGHrb8iKkNd4aq6Ez3O8rrM2Luu+eqhWzsz335'
        b'7z5kReNHrzBClw72B9vxoVmjiPF78MIPY7bDFYrJP48u3Y7f+7sPB4mO34eHv2coCTHZU5MXfnK+S8n8Ex/abYBd'
        b'0P7xB/Cv9w87jx+Glf34bttPG1tHDvsJTNayRur0bX7aHp9o3+EP8xrbBlcyba5jOQdJTZb24e8dHH5kb3N7f/1t'
        b'gL6HP/cHr1+79usDW+HR4Hbfu/5ClLeu76QHm9tD8qwR+NfuxvYuPqhdf0GG2T+Hl19/wR098Gttdxsv8Pz38NT1'
        b'9s4G8+fvTlVghgFG99POT4+YTDW5sjcrQjsa3jcaTgyOboT1A5rYW7yGNbnPVfav37rOdO3rL3xzOvL9A0Dh6y8M'
        b'qcDMaF2/197B14sL/fSGz3uz8TZhAzq4247fbVo0zkcZfY+KB1pWRyVsNK14HhkxvyF4pzM+YZq8wHvF0w8IWabS'
        b'dKg8UA1Fe/yqeDApVOu9s26KlU0XKfCgy9Ipqz1IJBsfJVMNPpT5Vdv1+gvKDnXL7a0hORWWeHt7NsnKGku5A2Wy'
        b'eRvpiG4xWimjdbHoxrNS/GpjLcYmcnn8L2TmaCitg8a3vLUNgzT9uAvP+NtUEkptfZbKss5dhs+QEiozTYQ+vu4w'
        b'SOuR+UShJt+9zvuhR9/fdE1G2Qj03IyUAWRbYmqqZ218aa2G4NgfylnptRCtFjzRnHIxJFevf/tb3771jKmF+85Q'
        b'C39RtdCPq0UwoFeysD0WOFDwoXYtegIBy4x8s+i7TFqokGySUYVaotNQIKhQwLTLs6gWYYFahOJ7zaBPLNNN4g1X'
        b'GbTHppOgVJ7pHtaGxsL5VeBjc1NsDIgR9dxFyJevFpjjBdXCPq4WWA7XO7SWTVSMN8XqwETBJFPtisfFDDo4WVON'
        b'XetOxh5ci75kr6ROz6JaxAVqkUD7XR4SCNlGoRWXwKoLtAFbzFvhaWNr4dGCzDDBJcgKq2F51Cb6KtRCa3tBtXCP'
        b'q4UJuvkUsfeDV05h3WKqxTuRkq3Cptx07kxRNKx77b23wbJIdupZBNXCs6cWUYgFaiFzCCzWAkMhaEsLmzpVX6OI'
        b'WivVlcRvqQ0HAAZuBtojeMqu4U960WEFahHVBdXCP64WonngCJZvEzzEMLE4wWSAmJzSjh3gMKtuTarCCJiNbqAN'
        b'OSV4mArbl55FtZAL1MJUdrUwQ7d0y7iuD8F0WYrw+FiscpI5dWk0xuBrDjApFSbDwZUo23K5fLUw9qJqERZgCyFD'
        b'VDGGYGNvuVYsYIIfCd3BzrGklAlZ6KyLwtJ5wAmsHHN8dJS9FPcsqoX6jlALKy+qFvFxtfAEFOyD4JtmTbnAvjVC'
        b'ascMVKGsyZiPj4W9lIUB2siyM13HGu8UvM+zqBZmkVoEZQQ+rdZsAwbatYH7KAEbCg4lYQKiGq+iyElJ2VtPTWjX'
        b'AUuTBPBQl68WTsWLqYWUj6tF6VE2paAPXhfRpII+S2ky3GSG45DBVAu9ARFxocNDNlVyl05HH4G81TOpFoviFg1v'
        b'AYCEzKMGB+2wuVlXmVpooadUPexxEtZHLx0MSvf4mOAM83fxnDMrUIvgLqgWC+IWGfvflyBqimzEleEXeuaD7NAg'
        b'FfSCrQJD67oZqaX1Fc7GaRVYORec5VlUi0VxCwg/GGWklK7YboIFTvfCMMUPYu/Be9fxsTazO0/WOUufWVcPTEW2'
        b'mFagFt5cMG4hF8QtvOxxuHrD9t2Z/YOcAZfmNajiSsWCgad3VS0cIZh4YBPlXFhcXbJuoH4W1cIvilt0USR2VWIP'
        b'JgU6z/4uBlRDNOHhp4P1KtUG4l9zcvTa0Xn2CPRs/CNWwERgmC6oFmZBlBMTkbGCkaoQhBc9AmC4IAGVtGe9TK1d'
        b'FaxCL7QWQXRbeRcHukQ8GtSzqBaLwlk2BmB0p1tK2ZtkoBnwEapjzzLn0VUYC4Udp7vJ4LI6WXA+67wtWhbr0uWr'
        b'BRTxgmqxIJxVGMmSwJexuuy0s81aWWquuWXjSsc/mauBx7IXskNGIhvfrGumW2eewShnlIviFk4HhvuaZzO/jt2l'
        b'enXWwHTg+4NmnlJNoHDRNnwP+wN1kRr7+NWkhLeXrxaRx2QXUosFcQtosK1VmhBBrbBQjtlosH+hMEsxVwke4lPq'
        b'Iipl2GMESwqXYjvgkqn5WcQWclHcQpdmQTHACpkY16UW1gl2c/Qe8BM7MsIFxwYcVyxQfW6yVXJWHZo32HgrOBMB'
        b'j7ygXiwIXAhp4QJtBVOt4FhVFWF7BvU2rtVovNAxFPyAa3vQFLgTTTwleiiOd/WeRb1YFLjA2KAS9CJMJU8ZoIx4'
        b'ohVg0ByjApaIVkVSVlGw3M2zZ2ZucCKeTSBWoRfhgpELuSBykbMOvueCRWqBDXQVSweAWcsIX1Fr6gWWIuiSjfRK'
        b'Gw3C4pP3QlgTZfHPol7oBXoBqgFH3HhC2ug9OnhHiTpjsSIjV9i82HZFOR+UxWCxDx34P7wOnLWNdRVH6+aCeqHE'
        b'Ar1wIlQbeCLaHexg0AZkWgE6ldREYpfsqG3A42QfOmZ2egV6alUpMNZn8VhELopokXPwspGG2xX4uowB+wiTWrFS'
        b'XkEbSlCA7iqIJHIF1QM6i75JrbDVyirshRIX1YsFIS0VRWHPq6xAuCECrKFRMgkYO2OTtaDWPaVUXO8OSxsywEWD'
        b'RoikK1sGP4t6Yb9D9MJdVC8WxLQg82gUjJlwJWULQ1iE84o51ZFhjZzhUEpMFtAzaAmbYtlHGQweXrQ08yzqxaLg'
        b'BTxwLvggK7GIVuWM71RA7lUnGle4FQGUDsKnfQsqCVOhPi5Jx+vLQawiFwczvaBeLIheqN4aWCpoKZVBQuyyQY15'
        b'dUBlsOoA3FybT9V5zAjQCSTVsvt0SJG35p9FvViUdeFJRXztBbYUWlCNrhgQVgpOGppBFJ+NsrmrpjTMRmT/wqjZ'
        b'lVBW01cQvZDGX5CnqgVpF9EaF6zirShF1AycLJrMbMItZbVa6AYAVYV2SRRqOHuOgYRByWT15lk8GlGLwhfCFA0+'
        b'EowbOidb0jqIPMMBp+C1KAbbq1RQewlcz0bX7GZfAl4sSf1XoBdWX1QvFsQvShNBeqtqCDB3NoCp5ljBxE0B8QKx'
        b'9rAfGF+D2bABqoVXsmaPCmDgUj+LYS21KH4B8NDA9LG5QFC9ZPMv7ZQS2SQbc/WWKW5Y3ArMpqJJsfOafDHAGFji'
        b'uhK9iBeMX6gF8QubqixdeKmq6vAkJZkMTNSDL6Z5kWgeU9HGltTbUOZABwlU2uFUlDPPYj6OWhS/kFimVgOHp7oe'
        b'LC3knYu3SYUaRQ0Rg9UwId2VKuGweYmeZheagfVdgV64iybkqAXxC8MicJ6nwFkYhd+qhXJo0GyXlHcy9pRDdIl1'
        b'HQpdYlfJZ4CpVmSVoT+LerGIpzp4h1hLjw0wHWC9CB0ytlCVBughw0xAUUxz2cAhC1VjZY5fc7GmqqEaq8gBdxc8'
        b'Y9dyUVpnCk5C24UmderNZ7g/w8MPBcjZoOlGOS1bUwmraGyLDmYDBjCBlKRnMX6hFvHUzpo4XfYWYmOoyumUlK2h'
        b'V7hmqUPPAq645+SoNA20xelSMQJVmbOxijh4UBfkI3oBT40+hVRbNlGH0EQVHW4RBFu5BkYSJW83BNU7HGVo3gNP'
        b'CxlNgNeBwsdnMq9TLcq9iFKwekoFH2kRJA5MQxsQEeiD0zYWuuYau60p+gDn7WEpTDFWFqdisKvgqSFcMPlC60U8'
        b'tcALall0x4uMs/h28ithc4XSNzxQUkqhagCOokqG1ywenqVqPJ+fST+yMH5R4V9UUSkZxc1UTbOtB6kco0LA8DkZ'
        b'qIVOXvGMoWtltMGQMzmLCauwF1Ff8NaIXhS/0AWDY/oedULBMlTd4SF5xYgRTtO0hTuB0WvYAZJ9S11TbK3sBZZQ'
        b'PIt6sSj7AliB1fcSGKlg9Ub4Bh9B/iOIXjFMZGKyZ+ChowJwx6KmnoNgqqRgcc1V6EW4IE/VC9IvPB0ggJBU9Ibg'
        b'1dBtBQ7uBHQfJq/K3LMpKTdIR4NxCyh9rcULXzHMZ9JeLIprgZqBkcJhyCiBHSoLsuFTfJEetrdmXaEIugPlyx49'
        b'9qHuPKgG9HAVf8QVXDIT5qJ6sSCuBd+H8VWlRbIdcDKyuLkXAJ+6QiLAmTk3V6TSAEeqOt8Vy48w0gsP5J7FuJZe'
        b'FNfKER8IHZAuSUDMzDiWbga2OGfXhIkix2ZLNFVrEFpTRanBYPVZ916aFdgLJcUF4xd6QVyLcUwAUpu90rxuaxLo'
        b'TU8smG4BmExgxb1aYBZzsqb7zGbuPDQWwsFiPovnI1p+h+jFRc/N9KK8nAjaZGSMjMnCzhVvWB2MIfGuEsgWlIMX'
        b'KWsqkVy7QRckJAWI2oUPz2LSr14U17LYUMIFYIwcIXqQktwj26jb1IA9IysAkOpDExyPT0pnHIMhZmfhwlcQ71RK'
        b'XVQvFsS1GtCRklIGsAwASWDNpETr8IfFW9FzNXAtwjVTbYrQ92qZtWW1AU0BU38WcadelJcDsF6zN4IvwJuc075h'
        b'a2Vnmxcs5x5LC1QRkwQP1qWB+wc34dG8jqmsQi/8BeNaZkFeDnN5XXSJ2YZVFQ+MnBz0u5rcSuC5WRE1upK6cBVG'
        b'ImTmOBty2haTsM+iXiyKd1rZaw4de8frpBWgZXAqVzYMwWfwwKl1lokrYLMJX19d9FAQLHGT8CkriHcqrS+qFwvi'
        b'ndJ2IIcMn5jYm6PmbsFL4TcsVB4IFH6E8S4HLjWU+xCxNYxQlly97PpZzPvVi+Kdubqci83GsyY/PC+wRVE5aedT'
        b'LdGCubLAhbCgJzC00IcID91ggHVsGMEq9CJeMN5pFsQ7WeU8dh1KMAztW9N40h6s8jAVsps2JIbDWqTOK/uM5joR'
        b'g2lZ9JTVs3ieqt13hl4Ye8F4p1kQ77SMZzGsFSpUg4UujIEdVN6k6gTMBTCGFTEZUBMYQOUxaV+9rbW50PWzmJej'
        b'F8U7wdMaTz18LdbnAn8Lr1ytNAbf2wu/RrvgpBe5K+UK/oCvwUy8TKqvonYSGw1fUC8WxDthGRi0FRFmh6VpQak0'
        b'IxixZsvaFgVAu0c4mo6xSBa2LrZZBvVKCTk/i7fN9KJ4pxTWSdbGiTYYBR1JJltYkOSD5/JFFaxo+GcD7IiyJxt4'
        b'5wLY0zFbZxVxLesuqhcL4p3GWiMBOWEMileyuCpETSo20O5ShBEheCBnFTAXlv2xrgceDfeqDShZewb1wojvDL1w'
        b'/oJxLbMgrpUjnIPgMY4soKnVAmRmkau0ABOyKN97VgIIBEZFMyIKqFFL7kHJxIbnz6JeLKyTkzTUwvbMtiSFhyLY'
        b'W9UocFcVUmpGRniZHEsyid4EEB8fH5vWUiWnVpCXA4R4Ub1YENdqpgZvE4YuErBG71UBJLms4QdJsqTqKgRtWnIG'
        b'NEVVKTFGMRSOY2Ut6sW3pnN6V/N57uG/8vq191zjz+Enpg2q16clZIeauh+aPjgUJP/brEZ//7TK+WKNhU/X2tZb'
        b'bXN7p03LkCt9p2+8sz+0ACn7rFM+WSSp49pb8s4uNHeDrYkmX7P2ta+//LWXvv7S619+7avTKuX/OjsV/Ftff+Pf'
        b'mDQM0AL+VrmaIlEYlsgCj3msSlXO1i6tYpmTiq2URMMck2LPM1mSVCYI/fDg8CNHZXQfaT77W++Z1bzfvdQZ1ja8'
        b'emh9cvTNnO8bk9Ydj9XEdy51ph0CEDFtQBdwUxmLAmwAeg4ml1yk9ew6mlgpyeqetXcxu+hq6ayJ/6H5MvK/yTLy'
        b'//f73vibrC38yulzm/Y9aENzjWkN+Xhn+klYRU5m+q+paH5rKCBvQrS5puxcSK11lxXgWnA+Qx4eRJtmQIukioFF'
        b'cMnomK2JkSFaWZ1kY9cPDo3rh5Yu4/9pfG18rR1+YrqXNlhamLI6/FkpsIsr7GWTTRQefoF3uRiE79g07AgBDM5m'
        b'NaDxvqQqMrQcwD+zIaHOEV/0ib0Gu7nOHtyTMr03v/DH3/ymtELc+uZpTbLXdpJc29ML6usPH7Y2MwVYn2Fdrt+y'
        b'WkrQoiCD6pFNEL1QLFYCc25LKC4n6ULNqrcMe+kyvVnWKpSWrCwhYOgNUPI6tnR7iwtQaK6Hz/7Wt557+IX/eBiy'
        b'XH7Ic2WLj0etDWPFXacAOxocy7fBggDAOm2Mgu3JsKzBqcIcagERWDzvWe2N5QHEkOwh8qmD/jvDoNVSg9Zr229v'
        b'td21tIN981banI7Y8xIZyTg0ADshYqWFUImlr3gXL/MDIp40EjSCRUs19iSwufZN+gCrwIzHUwf8Hw4D1ksOeOgg'
        b'sbaLT25vz9RCMRfLp6JziA0uPfP6vNbwhFkGcMlcfdQw6gCJrLLpoCsqNSV0LEZpHX1qspw63v9gGK9ZcrxsK/TI'
        b'aGFZCIFNr2S6Ab40gIHZSG+ttMAeBymWLHpaMo/pPTuNwNba0gOcocEii1DCqcP9T4bh2vOGqxb1tTjqfLnW7rfd'
        b'u/jsB9NRs7AeHHFRrEXQJN7qFNyn86qBQQJCGt46LRk7DB4SWDJraLiCmpOuC0yAPQxPbr3BRUyG/R8Nw3bLDPs0'
        b'c6G9dxhz717KEgOTpZpNEma9BVNYbEHD8RfaBB5rhCDxcQBZEAYvnduCuYJrnj7mvzeM2S8/5oX2Qikboc5aCPDd'
        b'0GrFlqqapqM2lqPrxETJsVJd1dihJfIipGXvnCo0tJo1IeXpo/7DYdThrFFvY1QPFoz6C7/w8hfGX3vty199/fb9'
        b'ipEaRiN8U4FlNVuA24TDMpnpVMI3vBeACVTdWSlD96KwFQ3sQ2TxtBJA3Ik6Th/pvz+MNC4z0tf+yldf/vr6F1/+'
        b'wpe/QcgzjFY7mAmXO2hMqTCvVepmsYYmdyytF1hVpttljY9wiSnqAVBIgzJICaPHvpb2jHUdvIcUy4x2YtSmTz9i'
        b'LbxncfsYsVipqNxMyBpDg61I1fRYYN1IdDVciNauqc7sxtiaUVx8VtvQQvZztp2Uywx6zlpMy+HPbIWhPWbmc9e8'
        b'QEiqXsHFimDHQzpuxQp61sKqqQiywYw2OL6M11eYFasatOf0Mf97w5jVMmPe2No52F/b2djam45VeiYQmqhLLEB/'
        b'RgPWuwoLUSRT7LKscGVBN5ivbkF0QCSyz0Ukn8GOhfI51n7u+uplxjp94pFuq9RjCbFifaGTgIUG6BRgAqqsW82s'
        b'52cCIFCqdnDTRTLl3FlYNBcUT2bAamThye3pYx4QhTSXMebpgBWDgSBd1TF4DetqhAU+M746DXNWklCGdySI35QB'
        b'gg0lxcRaWLl5y/xXbNbTBzwgCmmfYMCTFmNHgE0w6T9kz6p4IDdkusDisfXYjYpRtwATi39jSR0wEHxhZ7G0QO7o'
        b'RGHJHqdOH+8fDeN1y4z3Vw+299Ok693MXQiWvFUsbg2PnEVhgoaQTHHuGD38RQJqpouD9upa2ZfXVWi5LgCfwNCt'
        b'A2Kepw1++cHeB/8Eu9lqsy2nArQh66i9Eg4uxCt4DJ8SiIdm3KY22IQm4M0AKbHu0hjQG8+aFzwX0sAZoZ4+4IF4'
        b'yKX8G8HaIiscQoBLJiusbJsdVIsYoW3gj6EG2KzWvMssqsn6iBlbT3nDTce8bJvhAJPo6RwgIeM5QGKDPJK7Kj3a'
        b'RO/YjTwK441gTofC/heu6CAUdn2yQA2i52ShHkLZArsHvBAVRBxB+SsTDkivIiaVQT3y6aP+LzlqJZ541IPzm/aM'
        b'Xds92JwpCv3DresV9kF4yer/1VcLUBx9g+pIgKMUMehWnIW9ti6XBjdJyC8Uq4SA8GXrTh//fzWMXz7p+E/1g+D2'
        b'3JpxaLcodem6WGg4LHWBBuEvTEEN8Fio2IwGs4pGtqhgbyIg3aBGxZ4+gT8dJqCedAKbB1sEH7v7wNA7x5uU2K5E'
        b'arYi1h9yN0LLFt4wgqTAMAZQKUBqOGzLtFGmDDplsWeTB3GB7p+h8v/5MHb9pGPni/CZ+3vT7rxTC66oOS2Dr/Za'
        b'wQe5siVbm3RxHWuvwMOr96zkGlmGz4Nb5d66g//hSakvQLCnD/7PhsGbyxv8XmMDpJneeFobKrsDvDcqQ6GzYhhb'
        b'edhLy6bq1dpijRNqOOULKvehKq3oLOThMUl53ujt5Y1+/97G7tHgYXqhOIm9vgUPI4vVICvJCKAqEHOeOmRfYfIl'
        b'uAE8JoAqlhz/gW+VogAWe6pn7Nr/Yhi9u0TF2T7Y3b935E01sUpkTDuJAqTk4ejhjUByWsR2SBEEwuIxKLzGTGA8'
        b'oWZsQdKLb0DmpQMAnD78PxmG7590+BPXmg8AW/aP1Qb0pljwRiAn6IYkJEhgjtCeklwXrNcA6+5Y6RlsHN60hKy6'
        b'ZTUDqUAV6hmo+78eBh6edOB7bbOV/VYZGE27d7fX7rf9VNN+OjKaVM3rvAKFHSq6yyDq2neeWJGus459ZXk7KxN7'
        b'+TaeX3dhSkqVRdGs1cacpT+TeTyxr90bOhG/M0zk0RfPWFAIsEFMvkitJ4X39yG8WrOqIpXMqIRtqmAHhA6bE72H'
        b'KZVQOqB6gjelzgSRf855aHEZ8+ALW9rdPIl6gHphhxjgl1B01r4A/dEwpjz6YLIJQIV0EQ7OKFMaVC6EUlLLrkAg'
        b'AtQIDOW8GTyx/2XQeW1nM22tTdp2zgthSK/KQI8B6sNyLyVYnxj9sdzGGUxONtazFVUooHrJhFbfYxZA8ybDC7A6'
        b'xdmbWatLm8HM+QKic+Amt2q1s7GAwoWWpM9eFdCLLJuqgAesaBVcMdFYaJi3URCxle4Tz0HPcGD/2TDwc2OwQ5Cq'
        b'bAPUTHqBzy3w8ehngbfZBh4yfaQqrOEnfSjNJfZsylow1qmhHwGPAtqAgng4h1o8aB9WpVS4DsCf2vUZWvPfDkM3'
        b'5wfdFg4WD9Y26Tq8N/QhXm/vtHKwn/JmW98D8m9sE3h/Z8JcjeK109CdbXACPkbJokERDFW7Br5toeYSKw+AZzr8'
        b'A+AcT81q7114x1ZVwE9nOIP/eZiLvRQxvLuZ2SEh1TVJl6ALNKowK1k5E1jLqQewF+CiZnWVjDp7wZbxzQfRizGg'
        b'YAbO7oy4x/84zMxdhpTqdrnT3ro/NGRlc807W2Z9mJ8UA+peH3bOEGhQzILJybMUXLOQUmC35QASDDJABhTx8d1U'
        b'ExWv12de46rdwuQqPAU2jz11huv4h8Ok/OWL611MUbN+MCgo+QJbeIDqu1DB6YayygnuxuouMyuLWyO7SBFwiheI'
        b'hQVi9y6Vls/Ahv9gmGJYqdx2D7bWj0nTZFqC6T5FakBd3rQrHowUXA+gVgvFdA1YEmXANtNw9AwzLgNbw0Gg8JJ2'
        b'6BIXT5/WPxqmFZ+i5BZMcjCMDl4oSThPgEiXgde7cc4PLeAyUU6WtQVXU1Fs1QOjX6wkX5TGWgsCcM6WM2I1onP8'
        b'fR8fd3JShocVtjhRE0B9limxIVEAPG6K+SFJigobyXMs4AYhIDcLQGR7Mq6FLlzO2Z+z5Yx8GoI7dYoM6l9XkV0p'
        b'QXkVTw1tVICbQBIwGoK5NiolBdvBSsaua1kyZlxA7PF36U6eFc4enIBRq5Mbu9Ky33XeTVvl3vqvHrSDNkxMO7Zc'
        b'DHBdGHiLRgIzSAebD/HAaMYcgEpjck17mBWfmM9fk7ZyALDgDmxqcIYT+F+HmemnJbwz5smAOXgxdpcG7WfRKQBa'
        b'0IUKbBjhHVIGPXK8q5L8UMvMAqYDaVnGklKGqvoz5vmfDvMcIMnb27tv7u3AyO7d2T2ANr2T1jDQhWYCc1+fpE0c'
        b'JT0dL8Jk3MJhcwWH/9kORBcDGwNmKbAgJWnG+XuToQEVuqJYOAa6ptgvQIZiOp6G1p4R6B+Ois258EOeFjOaSG5j'
        b'i0kms+wVnl9KXoIq3BWwZrF0XqMGxUlgO2GoymlAeSRmwl3C4uaRbVyZOiU1ALnUItszwl7/zTDsM7CFUbOkm0fw'
        b'dgeAXvNQyDUAajAe1lR3YGciu9nE3MkjzsiEE4a/vFOg+IrRlhZgZqtMDkROsskHA63s5NAYV03OwORZz5NFEPAc'
        b'yxnm7e8PU/GrmcqpgUn4TMVzWyACbKkUYoeGG19YPCWmZOBrJATimsu16NpZKiVWMopeCGohYNvOmNX/MswqPAUB'
        b'3WZ/9WnEfqabMNjDcROYRgXJqIoHkIklQWxI+DggoSYr6ImLHszWVDaEtHboCQmddcYl4Iozts0/HqYXn6rQFk90'
        b'UnU96kyLBpGwe6gp4FaWPQGqri0zjisVYG6G4jomFgVWRcHvNmnVUixnuKZ/wpla8dQE+ThZPp6rDkCIoOlZKlAw'
        b'/NgajPKhWabFVmw2bypMu2EvYJO9tRy6cMx+hdgzOI44g4r9n8NU5RUI9axJQ8BiOEEIgIE59l6iLhlYn52xPSwM'
        b'PBqvTWH7KnaQLtjJcNRMa+xClMRWMv68rBurlvEAp2XdKK2ZKeRg/5LsCtwD3jWAYNrcgtK5S6IdnUBWGBjrSnTH'
        b'diamYysW+NzcyGrO8VpWLzPmMxKcjB8yjZMJQD0N7FCYzNofVsKruiGh0gvYykR6ye5dzcBKeBGt7d3xeBic2Jx3'
        b'6GuXjVucDLIMZabYS4p35WxsMmJLYxdIwQSK5MEfEkEpVpsVYlruDX84k6phGxnXTJHinCCLtefnvF1GkMXzinqW'
        b'venUWsqVGxtLKTQL/CuA7V5TAsqJFVSe18oFRJTZ31K1jiUohh1wzuZF1l3GXN4FT5fUJqVFNBhwZpZ5EdF5wbq4'
        b'gBPN2pS0br3TnrkSPLCbYLU6TBmPAeV1ex5wsH6lk1pEYdl3VAsdnHNMqITdgWXy0gF9suEEpurxrOVVAe89yJAT'
        b'8MBAfjBD7FpsTD5XVGE1szqd3/Gks9ZOnxhMZQ9PmDADAhCdJUrtUFERfOzeYW9FFqSEwLLmrXdWCJIq63P4nY2r'
        b'm9QZvIeJWKa74qKqrrPAD6xE00pNTlESoI5gbnl3vELTdZLZJuFA4ItlHmS2UZ+T8ebE0rxncny1/mi2kIJRxbcE'
        b'+LHBvXWmCNUYK4Mk3jCbUDAey1a8GtvHtxKq6UDhw5miggOx5YyD5787DFoulbSpTsmXZlFs4C9neiQliEL3iv1h'
        b'hwIgigwSho2BAjZp0nB3sMRDGySt4GhgnDH6czyHU0sq0EnP4Rj49YXeONWeRG0BNJ79ogD+oQYFOqF4ha7LCh1n'
        b'Mh5UxYcM/qx55ZgFK08f7L85DFafoRPnx6aBwrGeAmRQgqF02arJTmYXC4v0W5EBZkuUUmkAIaWhC57pFSCSzbK5'
        b'e4BFOi9/0L1bun6anXdM8YC14DUs0Zl2qRhorlRDwe59Cdu3VdhHLKIIvbRoaf99hA2xVQNsnIcanF1mrAvMd+Al'
        b'qVig1rrYZLG8KrBuXMfGwTpz+/DmdYUWO7ZFaFUC/og4BF0tiGDPZyU7ThbWvavBnh5YHI6R4BNrA5TR8Dz4buws'
        b'D+hYWS2DzZ/gfmTtmpXNmuZlL+0S8HfXwJkxt3ROWpvz73qspxtbz1ZdgcV8U+y2NI1dXT0AmncA5FKzMWZUIhc4'
        b'mRxsca26ELoJvLovCxyN9f48uxX+ab/ncTzi/YeH3398HWs9bW5uT1Ldh+th799p6c2Hr/zvuz907fCDBU+23Yev'
        b'fPEbH7l2+Knt3Y27THBcH+5j3t1NO/cevrL9Tz5+7fD7ppeX8nZ9sD65X/bK3/9r7712+NG7m9sZ79jarrxz9ic/'
        b'dHD40Z3dje3doCf3kX7ud/7WtWvf/PXrRzfFeJmQHWNKaTv7rU7P8tUaZL3JO5wbWxVPT74P/76PD+ZF02F2t65v'
        b'lwmPw3MvWFZwhN3da+tMXjt614gXEUdpq47aO5DRaK9s77Q7JUHP7/BC3Ghn82BvRE65gcmPpm+aXhL86dHW9qhD'
        b'9e6N7qb9NiqbaeM+L3FubPFu6mQcLyjYmsgv37+Hrz1Nkds7vClJB7s3KHRvaW8jb2xiFe7sbaWdvXvb+1DktLnR'
        b'N9runb3dcmd6G3NIl16/hzls9357lzOfXKdjDSB46MYqk6oVUUC1C1NiCm/V5NpkS8bqYmpyjlmGGaSUIbIO1qcA'
        b'pHoYLn0eg5t1zHBv72jxgrz+6NPT+6CC93RXIsR4VUKE4ViFCDGGi4gxkpsEEy2zhbG1FRhXLUmzMgf8EyBxE6Yq'
        b'IGJYr9AzyFnBW+G9vGJps+XEKFclxiiuSowurmQnbm7kExJzMMKd91cgIbBLJqEb521MzDMOgMOVWZsKJCwxmKKL'
        b'gyNkMWy8ByITy0lMrUxi8sqsp1qR9dyCm3urwfHsb2ENT4gPJLkA9/lYbfFR98T0pKJNbMQGwLeNLUXAsD0oBPuT'
        b'WYdXCNalDTGZupz49MrEp65MfH7oUXz54qttMz1odf1eS7XtnpQfpAYcyrRnBT5UTUkOTNlXVtPwmefduXhw62B0'
        b'SAbsD3yJoLWDymSnl5SfWZn89FXJD/yNSR+XLz8wiLYLlLs+u1X/qACNYdaTqCYJUHI4PKAVlXwJAvTdGe9EdhXI'
        b'OQfdJZidBJ10sjA8kkEuzXICtCsToLkyAUamLaxAgNx4pwivFMEUbOXZg8n5BoRSDA1ksKHmXir2GGxmZ8moANbY'
        b'NPhs6SUz9uJjXk54bmXCs1dmPbWKYYXU4ZS9x5ZI1bCXuaggxlg2W5QGr5cZiKYZpzLgTZNkm8rqwJLk8InaWqPl'
        b'suLzKxOfuyrxGRO8XqH4dtvbabeu4+HNetIDJsubyNhnBl6tZ5VDrTyKAXSp4IE2ax3Yol5YIT0vEMna2bTHW2Z5'
        b'dL+cEMPKhOivbA8OMa7Ll+FUeCxbdFJ2MbBGjOFdtDo0LcN2U0U5ERmOxi5UqQK8sHhUYjhbtOqt1kNdMOt9WU52'
        b'cWWyu7LQixVerGb/7d97bMOpmn0zgJfFJCW7lJp1MjU8He/OlqpqYT5w8KVVXkgPjtxGRwUHk31akqOvLNYSryzW'
        b'Mrk6v9JgyzDeqeG8k+r9jb095vM9KtGqjE9W1Rp7MIG1wa1ptshuJJxKVA38ohfFrad4+1VG11Nv1mueMEW5pERX'
        b'FXZx4srCLqx0txIWkfZ22tb6JOXqZOiTKXGZ+c4GImw89GFeXMmg8HW48AIjWzr5hWRSWWGMRufgeYld9b6k9NTK'
        b'pHdlIRjjlV618NYnAztpVHmE7yWsaJVSwFzKoXYs76fmJi1v9bTMg6/AlrWmFuYHw56KlhvbuS5rVPXKhHhlgZgo'
        b'Lj2A/QVeDb29v31/85HIp87O6FaSEJlXecARmMgtei2182KlDwzDKJVESHiFhXEtYOzJRC1MWTL0Is3KRHZlsZfg'
        b'AOZWI7TN7fLmI/uMOU/dOaBOA5NphzaoRJNZYetF6VKTKYYSQlDM3BAGRkGayjJuudolwy3SrkxoVxdvUfYCZwz8'
        b'/U7ZxcccP04qnu62wTSW7a2y2/bb+lD99c40SM3ipOkxui4r70EAZYuS2FyT59bNK7g1Y3pgL4eSsvOJl2BiMlmo'
        b'zBsypSRdcw3Lys6tTHZXFm7RFwq3vEvZ7dx7sDfUCeax98kwta1aRMd26i40GU3sukYXeKXe1hizg8yKLCmIqIrl'
        b'LTs2s8qi6ti9XTLSIv3KRHdloRYWZ7xsyRXMtJyk5onF3HgvrvsUW8+u9ypgPnkzHVvNBZmD80ApWgRRh1p1BgSe'
        b'6Xq6tmVRZViZxPzVARIhlpIYpLTXtvYO9gaZDVW21su9BHI3PdLbawObuzMUmh6O1U/CylyqmNSTtNI73qoLgt2P'
        b'FH5LQrMVh2Wbag9sIsxwjh4BPa1yBC9Lc4O4MileWYSFRVXiJYhxJ/f9IRa2/haQSp2UhzhxqsDKxqIPhZ+l85lt'
        b'lwSMYHPZYvPJCiIAown4EgBagFa6Lo43wKyWvIe35JG6WJnQrizAItklYCmh7T/YaXtTW4mVurO7uTO/A09KjB2R'
        b'mVuUWS+ygLTllHLzLXlwbeCk7sAQUhG9sBaidg1W3DhvmCpq7JIBFLWyAIq8ugAKVmQlWRBv7fZTUiAcmFqPhb2s'
        b'+6RkMyvWdukzi89rk/ECmZIykoXGMyv0WpOAP6vttZclc47UyuIn8sriJ241GRAMZ27S350uw1Z4Zysk75XRkGcq'
        b'zqRu2PsSslPAJZYXl0Vll0ATncvJBnaUJU33eVkZrix8Iq8sfGKiXZUMd9vOZnpwhhRZdbfE0JhlVLMT3Rpvbfcu'
        b'pOwj4CcInfM65wDnCCbhos8mZ14HFqwbuaQUVxZRkVcWUTHaqScnePVksp9PNeXCni8gByAAzjpwcJjF3rJOKmWt'
        b'qovWGxCFhEeLMRkWPVYlsxHLHpirlUVP5NVFT3hZ4DJ4XN+4e/J8LgvFQoEQRCky5sZkPtuNS7ykGGMSooALeF71'
        b'8SryXtlwL9ZVwJSyLNdWKwuTyCsLk1irJ/G1JxLSGbAxRiy4E0baHGvoRZsGEAKM0Zs2XsvguvbOJSMdzJ9LMbke'
        b'8WgsiRczl5TUyqIi8sqiImzkpp9YUkfFH0+AC7b7A0iwNtnmq5Ahs7SgCMOFftbLbVL5ll0Ae87W5FpT6inLAgeW'
        b'sltSTCsLhcirSzEJUa4qZAyGdkJuyhmhVAaCr4KNioX2WL1U4a2EEllr17vWBkzbZrinVmRpeJjVX5lisuz2Wlnw'
        b'Q15Z8CNq1he+kODealt1e/fO7nZ5c6/mxTCiOhF1KlaX2qK3LLdWZXORjQJaCCmaEr1U7GSWo5ZNttJVgglkbTyX'
        b'7JJJ5ysLcMirC3AIIeQTCGY9bx9gderJcL1R0apq2SKDqZBADUDfrDHhS/ayeld8tdZYH0JmkTjbAMalq521euKS'
        b'O0evLJ6hxNUBcaOXls9UOFiIdFJCBG6wYzLCnOkYqsyRNSgq79dEo1TJARQJ8JwVTmpwNrCErHClsEeyWJLw6pUF'
        b'LdTV3buRF0bii7fQxn6Ds9rePSGhoHXKkZfVhVFdsh1sq6yOaYVh5oBz7JEcgzG++sobwinYDFLLurnBxyUltLKQ'
        b'hFJXF8U13ognEdH2DsPsj92d4fVyFj/wPtnMXI1msVtkb8wDcN7EjgdExP4p3cNDpZQJD3RysIXLnifrlYUblL66'
        b'PfRkWwiftnVcfeGEmLSXJnrBYtPGdK96Eb5m4zS75vbA2/jJtWyKK1GbSBpbHWsuOCOL00smCeuVBR3UlQUdnFd2'
        b'aTGV7c2D+1vrPd3f2DyZmQFHw2I1rEcTpCoOmwYAQiaTQGc7G/u4WH3xukTFMp8aOykxQcpXLGhb8hKoXlnIQV1Z'
        b'yCF47ZcWUe8b6wf7G5uPoTnJYv9c96hK0F0FDR7ktAF28LCAOtSSUmuq9IL/92CB42xQiuWo3ZKnU3plYQZ1ZWEG'
        b'QC2/qsOpSavqtzGc7bdPJj7FpnkGlUUxVRobo4mmsguR9SrnogrbcQuWhQ8lxBycNCzs0fG31HZZC7iyAITyV3eK'
        b'H1d+QnWGIFVm05YcADdCZ+9UGW2soFbwYdibKYUc8WDQPhng+ZBSgrw1kDu2p0tLZvnqlUUkVLi6Yyr9FI6pzhAl'
        b'UD2r6QonlbXMEM1dyKRqyUIw3abl6nrRNQjVmsvJM1KYbM+efdqdXvLi9cpiGCpeXdmYC2LHxWnYcGoxgiWbzibb'
        b'mVVfXM+t19hUEADFIbnK0oyuFeOzF5ZJiKqKbLKF0VxSECuLVegri1UAH9h3JYqTydWGzXQb0beIQN5e12Z9D2xZ'
        b'ymp47Grro2UODMtPSVetK+RdgYeJ3iwJNMzKghJaXt3x4Pl+6vbdDUx5+809VmVkNeD7G/uPnKb73rwTUojgdKsS'
        b'OyACh3e4lZ58EsrlwEal7KsYneF+adkJ8N3cdXRLXow1KwtA6CsLQHgbVgb7huJ4Z/mZ2FvQGpbNOvbVNjBeohfZ'
        b'og9CdLgUlgfVXQsZYzNVg4p372sIrPkmlqRWZmVBCq2vjv2alWO/86QpA2xd8b6k0CA5FzKAA2ykCk05a4KUGlAh'
        b'qah7aL0xyGtSyzClovlWl0TyZmWxDG2u7sbz0wCA58lTN6C5EG2QLiYXwbN1ruDTsXnvU5IhgZp1myUcona6RtaI'
        b'JVqxHX/UJcPwZmWBD31lgY/ow8ryPoeOoGeJ0aTIap6xqGpssdIZ4WF1fWeD+1wbCFsSQmDDdnA0nyu3qGE9WBk8'
        b'6PeSYlxZhERfXYTE+5Ub2XPFKT27A7E9SJHg2VaIxj4M8KEhNPhSpkVplYSmBQZIZYZUlRlWN1lb+rI4dGXxEu2v'
        b'zso+jWzQcwUKrWpRWQqxAqk6k5X0qfcGpCqFCYalgWXILIMB1sf+RL5lvD44wavxSwp0ZXETHa4uYcDblWHZSe/i'
        b'swRZ2OQHtIOtrZNJvkjs0iiCVV0bLx2rkWs2++qA+yIozyKf2bDushdSL3koalcWNdFXFjVxQazc0J4vT+zEDEwD'
        b'4g85NTakE1kYWQs7IHdpGQqTsScAo2CkckXg2QC2ydgZuOmS8lxZ8MWIK6zg8xQs7fkSTQp0kqU/U6k6YxeyMRXk'
        b'2kOUJXcbmQipUq8lsORIFWzfYLyy+JcpYsm4pl1ZDMdcXQxHeilWZmo3+jk7s8MlRoDZ2qQoFSgVHFK4blPumeG3'
        b'4KqQ0UgT4CF5IbQW2Vl4S9Rs0rJyXFn0x6jvZkh7rjhtSIFtS2UyCjY0tgKXKISCH2X7Qqt9ckO3XWOTN7CxIWO/'
        b'4nUiDY1TlhTnysJARn93Q9rz96evymerlWGvYB0qzGyXSlgnfY/NOy9UB5rF6yrb48gso+y8qcjeXnFJymlXFgky'
        b'V1iIZEX3DinHvY13zpZj1+AbJbE3u3IRCMjkonNTkBbkZ3sFARVW20nfEK+KZ0XtoWVIzjktGWW3K4sAGfvdbGfP'
        b'F6cRxqdGKmJTCOSWMUjjYvdVB8P+AyyubUIxjQ1WpbG9eyNkYUJ7XjLz2a4sEmTcd7edPV+gWcHOdl1KqEkm25Nw'
        b'MLfeR2eHnlhO555MVKAjhg3PHdAS0AbEjuEuG3G3K4sFmSuLBXmx+tvde43dm88UaDZCSNZSE0NWdQ0F/DJ1HbLr'
        b'zfUIa6xCs5B6S1bUorA3W2K2mzYKgl1SoCuLBZmrKxqrn8YRykVEaitMKA+eAXJjgnvkPf0eS0oMvKvsUoZUlROK'
        b'mW8Zhribih0sS3IhLHmK4lYWFTLx6vaoXfkebRt3750tz2CbLJZxIJaukdh8slrjwFs02+yKbOFhexchGKCi5oro'
        b'IXrvRQWzccteH3IriwrZK4wKhaewRc+XaBG2haRFTcVnbMrSQwBdsaAlvfTqumF5UhjiaoRrLmljcvbWDScsQS67'
        b'Q1cWFbJXVyPFrz4DdWvjHINrbBY5U2pJyaBlKSrmIF2uBLhwn6KxKqZ0wEhVYPNia5ZgXeswxNkseaPZrSw4ZNXV'
        b'bVD3NNIQzhNo0ECrGS6ypaxEYHHumkuunaGhwuobWTfs2ZpVC3Ck0TegYatTKKVgay8p0JWFh6z+bt6f++cBIhG1'
        b'U0IZLXItOsjElD3VtWMlleJqbCWwlocrJkRwmYw1B3WxxQsfim5LinNlwSFrvrv357kCtaCfKhtmkiSY26gib76L'
        b'KvFQ98BITWpRDPxmE1VZMVQdg3gBbrMMcsnwrVtZlMheWZTIa7fy/YkxbNQDtnzd2N3bP5mQmWX2KninE3O/WmNu'
        b'v9JDcyCnTPj/y3sXKEuzqzxs5pYkC72fIASyW8hIY6GaPu8HIKmEkIRmVBJIQhJgqTjP6RpVV7Wqqmc0BJDAAkT0'
        b'YwMq4hAriReOFyzlAUmcxDEJjhPydCAvQ3ASYgjEVnAwcWIbxMPk+/5bVV3d/d8aTdWtYlmjpTXdXVV9e++z99n7'
        b'+87ZZ29dTewut6qKJ4tJTjngIMvU6mo745Wnu7ATIvtHd0KkzCXszVONKWSOLYtgpAgaOVPVlEFBWJRQinHJaBaB'
        b'cXSCYR+lKHquEZkVP2cBb886wOvCTofsH93p0MUdJty9L3NUdzbpAQ1xMI+3TesUZdNdJYXgCRtmxN6kCihnzF11'
        b'voE0XoccwEYDDO2RW88KaS/sWMj+0R0LKXWZ23LClqYIkSIAUHAcm25iElmZbAp2n41KytDYisk2bFsvStHdR5Nq'
        b'F1EU2PiMIdZf2HmQjU+KXdmsuLPTDw93Uq7Bcah7EtbAWtYHX3ICFqoV1kjchNijfHWckCSdMcJE6zXH2ZzRkhd2'
        b'EuTEk2RXTthSxFwFTJU8KEcNuRgLssSZeqrY6iXbnIjQK0dn6OaUDSLB8r7hO+yHdkZbXtgZkJNPil0JNBruZCPS'
        b'tWa0LxyKGK2xWdtqldRZ2SKiT9H7ClJpYMVmdc0xqi4TDI7cmf1Z4+uFHf849STZlRO2LD4EYJmivEgh2i660UUr'
        b'n2vt3bggY4m+2xJDSQ4+L62URQswznEUwBmvw/yFnfw4/aTYlSkkdefEDWRA031izQjBT+k2+F47Fp/9VRovxpQD'
        b'BoomtsBSoOKjiAaG17Lns8bXCzv0ceZJsisnbNm7NC0o9uiwNoeYY5c6aGUjUJDXQLFJKu9V1Pi9Fq3FIpqSLpO1'
        b'pHDGqiB/Yec9zj4pdmXPWd953GO7Cz0o8shQncrW6c5pi7Csk4S2UYekS+ieYwHYNBGbscPGEg5w1tEO/sKOe5x7'
        b'kuzKCVua4GzzphsZjcvRu+xcFimrrJvsUVTAWSGk7hKB1yk2KlDWhtKSUjWqs+7KCzvtcU+O0x6P7Xdn9SzgapEA'
        b'MFG1StbvXDH4FpCPyB7cMlrRjPBGpcayTClAMZEti1Ix9rM2/fAXdtrjniynPRO2lMl7bU3JXWanG9IgEqoUIcCw'
        b'GSB2rIlOPshejFesyYxdR6l0t7aWesYzgnCW0575WMLHs+Wdpz38R7B2m4VW2MEPXdnsV/avbe4dWjXtlmtw+iu7'
        b'OzfxA7vtQzc3sV5XNvdhwfYI7Qrrbu9vPXYl1XoFf28fIpe0dWV8N4kPaeXmPtsnftWVG7ubO7v+Srr5YVgHol4Z'
        b'p4LhR6jW5s72lZvb5VrafqjVu9vwCGOtVPbE9cq1nevtat997Gra3d/sEHWPNdGru2xLq1eFTNLInFaBVZ0U0l3l'
        b'a3mjxh28yh+c/3n1EXlbQg2IuBEcpPnkeFkJWOsNmz8jBmuEWYkd2jz+o4IG53QtIdXKDoZqZQwLi4Nurcux8ebm'
        b'UtMOcGH29+KfSfsTqQqOpDq3/ef7frEHsEVJlCYTIIdUfYkAyviEZlsHQVUagLkjVAMX62Qq9jgvuaWrNWt9yvPs'
        b'J+oB8jve/4yD4UXjmgW9cT3dwIo8NHZbPviOg/uGFfyRv3n4U0+5555hfdFylJ3t/V38ab4gYGNHC+Kv7uStzYfG'
        b'EW+rh59+/yP2/of3drYPHv5X8KEPmLc9556H/1V+/JrMynhsQV2FM84Idmg0gY1onSyt2x5bLYAkTqnWbE8pOayb'
        b'LDX55pLSBzdvfeSw0nb6wcdvDs/l8Lmb11vdGKsw9w4e/MUH73nwnu87GL5oc/sagvr+0bc25qv32MGwhhici9RW'
        b'GvYrawi/SKAG2bRKX2M31gnXQ04lSQRmeACYaZDSASRL0BhxMLy0tt6wIerGjd2derNwBTaQtMoH97CubznMU/PN'
        b'lrghHt1EMqOvP3ZlvpP25zmLqQ8mfYgjNE56PjbMVj0YXtZv7t/cbVfmzY/Gn97e2b3On93Z+eDB8Mr5BuO3+b13'
        b'vukNX/tN409tJW6Eo0x5MNy/s91Wb+61K0dDTK+UdGP86PEzx3GLUl8pDdtq+6G9g8GPidXLeQbebo9G/HZ3Z6fv'
        b'nUjXb3rHm6+k/St5B8rNNwX+5tv6uNep7B4lh/PsbO295gqd8kbDf6B7v7m1tbp/88YW/G2MELdWip8+ThS5wtB0'
        b'MHw1IMFjV44mV1zZyXtt95HR5cYfJTbYalhJfjcnagjBj37kYP/g4a9/6j33fP/wVHr9YwffPzzr6G9s9M19/Pn5'
        b'uzcRuq63jVSvb47jJg8+Prwo727Wh9oGFgq4g7NNdh9qBw/86D2ze27ed9/6p2fD5oLdYsTqo4huq237kbYFMHO4'
        b'V5S+ii3LBV/FTwIorM59ROqI4HH10CeiOPzyKla/fVub76T1v/3GYS0J3sYrRBDZQ7XAe9LpXGWC5xYACqB49ujj'
        b'65ymSnUN7IzTOUxlu53uDrAlnt4e2awM4gfDU8ZP3j/4noP71v/12fDQRWgj5er+zs7W7br88NOHtWgkn2pyIl3I'
        b'zUWDHZZzTEYW9tDVIQqNTd8FTA5KyhfV3SdZQVStkkVCl4d/B1Z9+LP4z1yHf2O2MH5Bh2PTzkMYWNFRCAtX99Ij'
        b'SIf7bXt1TCWrR/Fr/e88dVjLMke+d7K6GZuUxz8PV9VJ6C4ahANiy3yI2nQuPZvEmZUQvgThkxGiHEvKyLUPOf/N'
        b'2fDg2eTcbWWH8cO41fl2vX9r56GD9Z8bsJ69skaqWK2tIwFAegF3QlAFkLXOdoQuPdbBeWEKb6VCValrLYtpSbQ7'
        b'pPxeyPlvzYY/fW45j3yZuxlwXm7Ms5a4/8ZjBw/8k817kRScRdIK0hqRRSwlRI/lVUm3WIO2bFKDMMzBoL6VUnUr'
        b'sqUQXJa+K0BkT7d+2vzfOTgUn+7wk7PhHWcTf7627DYHiLMJRzh2iAce/QFKbJCpWzESvmujdV0l+HANstZqm/Km'
        b'5dCi9BxMm70NUqfkYy/V266kiHXCeX9qNnz9WbEIf3IVuAoC3xE1fuylw1rLMmDxdFYqJGGFQE5tGcRCwo9JJRFV'
        b'cmf3kcy3B46NdpXqOqQIcCLzhLD/9uzh92Od17/zpx7+Vvx69w/8O7MhL5FowQzM3uPm3AC0qzu9g2AdrG/uDms+'
        b'GCx6il61IgrwXYnsj6mEjrkiBCZjdUEIdE7ZUkCcpVE1dZeSih1Ujfo9+2nHnk/x/93Z0JfMExeqsHcvNm8G5Qsm'
        b'WtXh4VkrEP9akhaMjt42FiybqopGzGSzEFtDAc7kJGMVq55S4a/Mhm9Yogpbm3mU9qdn2K8I8Yh3JkK6aBoQbjLO'
        b'WwCkRHEbRI9guMEmbNvcOK0DIQg7G38H4ooJaf89hprlSXs4OvrWAMaD9ee/d1gryhRvq4/VFh91T9klUzRHx7EZ'
        b'hE98tQrHd74iulhOaMRPCAnKHpCZ6oTg//5seP8SBa8N+AmI8hqACqDTKPna9WEN8vYeZbNCITdWA4SMOM9HmvD7'
        b'XJOE/L6GFgy2relKhCa016mzE43TU5L/B7PhA0uUfHN7n4Br6whij6L/5lcgVpomjRZ1LDBzcG3sSZU80qPp0Rnv'
        b'RHYV8SwH3UHPkOet55snRCDE/GYmRP+rs+Gblyg6F/s2sd/zNvgKQD4QifJequp8wy4EVoE7AGXVjFiOdYWH9IKd'
        b'GbSOYHHVgbqwPFL7mCfE/g+X6+SHEeWk4LNvwXo7H1w1uncpqvD4LcKe0j2OQ45iM05lbGDgtIBNaseSzgzf19Ya'
        b'LScF/2sXEs1326NptwIPtK069/S3/iQwLoTNXFsD7+1ZZUCVrhqwX6uI7RbsOHTjeanPqozmAQmNK4g/ATu3+wnx'
        b'f3q57nIoNwuh52J/4gsQwYM1MRpEj8LRu0BiUquiHHuNeISQrFLF/rQa1NnB8YEcPaCaDz4m632ZEPs/mg3vWuqq'
        b'71+by/vzL8Qyqwo4ZRA8iklKdim1MUlr+LRXGUqoWhqCSQCCrDnWHgAqFWcrAPAgh07I+x/Php2LSZrjWfCho1w9'
        b'Bm6jLn/4gmGtKuOTVRW0PRhsTm+sabbIbqSNLipgANuL4nqrIryU0XU+HvHYFFVEOaHLzyzXZdIeSO/GjZ2tzTLf'
        b'qB95PXBL7ZF92xRrwGrrygO9NCJdJZDfOQPOxNIZNmVTEJ2JVufgETOj6n1C7L++3I16UuyNOcCee9AeV72Ab0i4'
        b'TJUgg0rJDOiegcGQ7aUFhEx8LQd5k0ZIrKULAjA4fsZmbnHKg/6T5WGWW23UD9Y/+RSQOISVWoVNJXa+xhSycgAp'
        b'QIow2I7Aj8UFjpFQuTrgRyBsfMNJjujLJscJaf/GsqVlp/GDB+Q7yTCqF12AwCGmGF7Qddv5WA1IXWHBObqpgfYH'
        b'vp1Q0C1BWM8G/TJloF47lTX/01PkPds4ytEX7n0fol8E7XWluoDMCCsnzzaGUpSUAUaCB9w2WHDLKQ/YotFpiQzF'
        b'gwtkTGunfOE/mw3vWfLwTB7ZU+QHdmdcYqt4L+q6JZf3CAbag8JXRebOkcLIigreDQyiVfQIKMKwl01NXgpOWZ8Q'
        b'+mdnw/uWLHQG9dsaxV7/nqvMMwAeSXrwYrB5EDiRSldOCV+Ewvoil0B0NkDsDcSgZfhFSK6kaoDCvJsQ+j+/AKHH'
        b'E+lRaC+HNVk5gt5GrGtqunWTg21eIa5h7wUkmQZHcT4FAETg7MwR6EGpAgZUcw1TzvxfzIb3Lknoo9PQje2dOl/o'
        b'F3w3vMNWcC0nOUchNBl5H1ejC9iXztYYgfOEK8CpQURVrBYKWFzmLCrIkbdTMOq/5IHEcmQuELWMsv74FThF8ibJ'
        b'DhYGmBRbz673KhBChGoIEA2RLYPUID5rEUQFF9MZECoLVyscfCqT/FezoTwRWXkE37b3bu7dOQ36iIvttTFpXyVs'
        b'mrPgefi47xUIzqWKJhInikjvWotj1Z5hvzawBl1AcW1yOXs2xDAj7Y1IN9jAPiCxTMn/Xz/BYHe7/Ddyn3fE3gCj'
        b'2azjMfIYOew9jBylWgc/kCAIWjpAPbg0dhyPqCxWXFbkPfgI4nZAtEaYBip0iHbOaiTMMBU5/pvZ8PYnIu/+Yzfa'
        b'3qFnwBWu7m7duGMG98H6z74RouqiM48+shIWOwqrmBLgfkseaMLG2J2KDCOsA4FfINiB0/OGzAMR2ils9Ddnwzct'
        b'uQHVyXzykTisucweYDwbFj0HZGeA6Q6Q6nNECh/fDSqZkgJZ9FANVI1lSYgsbDkOzjsh9X+7XGh0WxHHSfE/NRvW'
        b'GkAGWGHyHlwQqgBbGGRAoGcHsUEpsw2OKQRZnGzB5QT2gq9W8s08Jf7PzYZ6cZULJxX4rXuHNQDOXGJoPAGp2Ylu'
        b'jbe2exdS9pySiwDtvM45YBMgQDrwCJORHIGnC1D2hAI/Pxted9bgd3T49LHnIC4TJLMEn43/kPwQjC2cA8FZ0PwV'
        b'+6t5wFAQSNJfDSfWLrILQKgJuGNCtP+O9wbLicvXd+oo6dufOaxpHbNuQO2ctx10Zy1Ww8oBQICeGI2EjEhhFAJf'
        b'41tKy/mbVjiwFnhMngJF//1seO1ZJa3zNdz6NJhHqonNTw24REJucNYhI2Mr9ZY1gHvWCiDYeqygT/hqMSZL4/ku'
        b'OxsxeUTwP8yGN5xjDfvmQ6N0AZs/If93xHpIwB7mfJLYAICNS4XPwWMSomAlwapb5oTggl1VA5Ac+J8rk5n3f5wN'
        b'bz6rdHdG1af/EqJqBNq1vJSy4MahFw2Liq5IKbTxWgYHNOlcMtJxUGeKwJkRX40AOlZNJaz/aTa88awiHl2Mzg9R'
        b'KgKQ0U2yhy1fkPkqZACgCkWA7QiO6AIukIrn/QGZFLA818rHK1kWXhfmKZT4t2bDN17IMPuD9QexWc47xn5C4l+Y'
        b'DfZMU9wPHnj39ZGDnXOC+4RMv/h4G/jxBpgfrD//VQiC5xxdPiHZ/zwbXn/O0d0H67/9PASXcw7tnpDtlx4vdzzu'
        b'zOqD9R9897B23mnVE6L97bMZ9Nas5oMHDj49G9no+eY0T8j2v8yGr13GnOKD9fp0pLNzTiieEPB/BVY674Teg/U3'
        b'vRSWPeds3gnh/rfZ8FXnmE17sP7yL+NGPd9U2gm5fnm59zYTEyAP1v/gJSDt55zHOiH6/84ik8sZSHqw/kk413lH'
        b'kU7o8HdY9nNZszgP1n8TaPy8UzgntPiVU66iznG6toG/M+f1D7z8N8Yztl4hEqhOQ1B17CkhJVtLEPtKIYX3gltX'
        b'WmATZZsFl5a9WOR6mMKoKf//1dnwqs9xgubB+qvvZdQ/3+zMCRn+j89NhvmBrvvoWDJyzrGRE1L82mx4zROZm3jw'
        b'4I+A2p1zYOKEHL9+ylnREgcGHqz/pZdhM5xzVOCE/P/nbNi6zFl5B+s/hth03il5E4r83dmwfblj4gD57mUt7fkG'
        b'xE2o8veWe96xcADTwfprX4nNec7RaBMKfGY2XL/U2WAH63+ZXnXOqWATmvxfy7/XfbyxWAfrn4VbnXcg1oQuvzEb'
        b'2qXMgzpYf+qrh7XzToKa0ODvL3+PP84opIP1n2DR1zmHIE2o8n/PhhuXPQXoYP337mX1w/nm/0wo85vLD1iT4zUO'
        b'1n9mFdY45+CbCQX+wQUGrAWaMGCdd+bLhCa/daEBa4Eu84B1vnEnE7r8P8t3q8lpAgfr3y6HtfPO+ZhQ4B9eoFst'
        b'0IRudd4RFxOa/L8X6lYLdKFbnXe6w4Qu/99s2Fi2W11LfAt1S/YH3vFZHmxBSGApm7VTxdSaXJIRMFH4FMFlfUsI'
        b'xapWA5qhQbk6yAligtWAYYkWnJD+Hy3fEo83nOHgR/7iT90z/o9FHmvnnc8wodQ/ng0fuvQBBQfrL6Y25xxNMKHN'
        b'P7lAmDLdyf2ABVlr5+3KP6HKb18oTFmkzO9j45+3If2EMr9zgeF4soH3wfqPwyzn7cU+oclnLzQcL9Dld8fzpPO1'
        b'IZ/Q5Xcvkiy2hVY5bwfuCU1+72LJYltolfM2n57Q5feXf9S7uPvyAc9k187bd3lCiz+YDR+8xMbDB+vPomeds+Xw'
        b'hB7/dDZcu6yeuwe8ZVk7b7fdCSX+cDY8fHntZg/Wn8HYe85GsxNq3LNyCbY47M45t8V5e6xOKHHvyqXY4liNZ4wR'
        b'93ztRSfUmF2GLQ57Ms5tcd7OmhNKrFyOLY7VoC3O21RyQo2nXIYtDjvxzW1x3n6KE0o89XJscawGbXHeVoITajzt'
        b'Mmxx2H9tbovzdtGbUOKPXY4tjtUY88U5G8hNqPH0y7DFYdetuS3O2zttQokvuBxbHKtBW5y3bdiEGs+4lNwN17m1'
        b'L3jglkothlcEvHx3qbuC1ABsmAv2RwMSrIrJMfMdOXJGBxQE9gVyd1OE6ZmXlLuP1Bhzt1a5sIYhq8AqEtN88iYQ'
        b'sDaZgW+5xVkuG6xHehQSjF2xagrcUCIYT6jxrJXhW5akBrus7B1X5EGLscHGNz77qffwflZVWbEjRBOW7SqMytlK'
        b'JVwIFiwW+1pwp/vWsUew4SObMBkpwUZYojwh+bNXhm9dquSsHNm4U/wXvpwniMhlIovaxo4Urlb8tkqHxBC8ZsV0'
        b'A4VIvD5zFfC7lKgBZA1fwWWjo52iqc9ZWdpT7Ln47Gxy1FmHoq///V9DEEpY2eAhW0ZwQcCPJquurM5OVIOob7UD'
        b'kwBYB1kFoasxmIIo6mAfMOyphPDclSEtf93vkv67PoE8EJ02PiGcFNdybS7K5DwijDEIQnzyIrhni7ORvFon1ZE2'
        b'dPJCJ74bnpD+ecv29zsbyhz86Arf5/Cg1dhugIOELGA1ic8NrSg1JgX4BtopRXQqlA7XacIDtGpWKVrHe/4JyZ9/'
        b'If5+h/jrX/JDfFxUiq+ZG9Uk4AloIPjgkM5knQUOahG5uMnaVWAFv+ulujJW1LJ6Z0L4F6ws7bT/1rLXNraD4t8c'
        b'Zf/i70OIhL+oBELQsU+Rf230TUPGqNjo0MogNdylmyChGqBo6qroCrzdJOjzZIHtC1eWdgF2x8LfqcCXvBU41Koo'
        b'bVWhgzQ6r5Phmb4urByEkOOxUayhsOeAClVbWdnFojMZYB9PRcoXrSytOuoun9847Fs2+v6vmXvnsR48AAwNYL9U'
        b'ifwpleHhWPIKYALf6Fnq6gAjEB9F0CwXbK0iSSunJ33/xctDDNO+f5sa3/XxcQs7zxfbAgwz2DA/6ELcLBoJy/kg'
        b'QgSg8wqbG0AngXaCIwdXFPuJ+DzZquULl63GSQ34fnB/92bZ39ndG9Wo+1RDC8BJQOnCxBUkAmbDuiNpdYV8Vhk/'
        b'DduegPd4Vnmx0g8buSDa6iomKcEXrQxtqlHUtlnFMo5NosAs2ipSeV8FjBerAYhrFZGhqGK0jkYdNZCSY3ep8Th1'
        b'dX6cujpHQYfdo/7w7w1rILoepMaQzIMNBy2qdUGwXAgRCm6UIl+gO3acAwytJWZRMhtgRFiruTu6R923/pKV4d3n'
        b'6dL2iL06v2pcvbG70ze32qo4FPcVnxnWgqhBs8VDK+Pte6w8r8saWxaCAivUolMiBgVINuSSIDHwoFLc+Bz8DnHZ'
        b'Bu2LV4ZvXLDgx8251K11PxRXXT1qDXrKAv8DLLBoOgNAhs5i3ogtGiMcGrjeEuOzsKN1QIIi2RVNFyy+tR500Zpo'
        b'a55fJd62wC9d+gLLQ3Ff9hle54JeyMg7G3BXhWXONWoFs6uYEAf5QiskrDs4rwmiRMAcZAKNtCYB7NvEAn/JypDP'
        b'ssBj6dh8dcfFPvHnydV+JsQHpSAIMwJJs/M8wXgLAAD0Fb3qQJlg5J5tW4BiQEZE4NujKJzDjiw2prtW+0uXvtrq'
        b'UNzXQFyebigwC97ndxANcFaJXBl4Oltc9s6DgAvWUkabQzOta1n5FFPbiGg4tdovY0I9w2rP66lOLPfJL0yu9wug'
        b'QK4iRtILOK6xPJ2N4EMQjQcjICFIn9JUkQofv3jD1kXWwZG8tcoeXsvctt5/fOnrrQ/FFXSPVuELBpzUF8Gn0hby'
        b'BQ14G3NUoHXsz+ul13yQgzgCQAAIIGNMxonDh693rPef+By8W9+13myu2W9f7lt/XujdIP+S74gAW3wJAfmlIFT7'
        b'xi4RHiAGSAzZkQ9QZGX5Zyd8FNZUnhr00u5a7StLX21zwrvhxIRWCZRaFAdfTnwfHVQtVnF9kRZ9DdVLUAtRjEL8'
        b'wGeDf3c+Vix+YrVffsbVHstvTqz2iT9PrvazIL4G7OAdRCsNO07l8XAvN7i2ltoLGQv2IeskOqsGoxF8ZYvQLZH+'
        b'yQvvWu0vW/pqH/UxXf0MK+et4ctFJMQOmNEsMiWP9+jnvJZAzOu+Kw1qkapKSXsebZRelG0iijix2q9YGb77lNU+'
        b'FIbiaqvLapZJrHrZ9GqyTq3KqiKba/Qc3QlLzCtVTtri5FcmrfEgnclaWfApPWCnRiedimT/VXjkJNbUx8zeqIpH'
        b'NsFIQBVET5ki9QzF3u37f3Lp1nCH4n4DxK06xA4vNiI3zeOh+Ts6RnfQIuPZgibS44ETZe6Ona1U7uzoYl31csIa'
        b'X74yfOw0a9ws89bge3OLeKdXHfxgFcxLrkaX6irbHmkhjQ7C3rLIvETlhEFOfmHSHm+HgjbydY31OiImgTNhnyMX'
        b'YLNkWVoE+1OAlKWBIppWQtc9KrZD0OBenv2B7rLHK5duD38o7jtpj+5D4RFxMxpbQgrLBwrOQIse9PhQB+Q7YSMI'
        b'dohSUo29ImsRfIJc8oQ9XvU4u+Os9hhrU06Y48SfJ63xdYxVQfMsKiJ8muLwe3ZhUh1kSfEIE8gY7KDBJN4COqdo'
        b'Gui4BU0BZbDV3m2N+5ZujXDCeVi+CVjDQlnB3nTKqBAMiHirSQTJBmOhCh2qBZRTfHWogNCEZLfgdPhm6w5r/KkL'
        b'ssb+HdFqvz2+NRB0U8rBS8VaNV8MX6Ml2RrSYcrUTdUqE28hkcrj2AG29ARu34msxV3WePXSrRFPWANcKYO88ikl'
        b'1pcHlcHGnBzyXAIHh4glInixfwBoijBKgCZiocZfscMnrPEVK8NwWp4+PIGf2yJot+p8Ap0FxloFOlSrgAosPPY2'
        b'p3CkjLl6/NfGapSrt33S+KVT7PJXf52nDryHlzAHb7K0MbJqKE/4HZtGZrQNScZC0+ijajrR5Wpld2Gk1Zzvsstr'
        b'FnDHc3CxI7b7VyBv5oNmzykjwrC5ZIBgAmwc5hFs+hkKlk5bEnjpgWhldGzdK2xrzSDyThhmdWX4sxdomL1Wdrbr'
        b'7ZaZf+0U0/y1X2fT4RohdbTsDKbYc0tXa6Ntnm3BQ2EljsP+rz0XibilC1tJ2DI2I7TqbuJ2//JNI0+YBvQ8deGt'
        b'HiWEiECDBnvF5VwBVbxADvHAKt6GgAiLzAg4AjhUeQ5a61QEu3qxe2akz7db5vEYNfcMuHJOzWbQBxAl7WwL0WD5'
        b'pYhSdxZ5B+9kVGzdhG0TCiuqwLZ5fFeSMXcZRizfMOqEYeA9VoD+e5+VY8eTMv4JZNmwVbWDCwWbhVYC4Thm042p'
        b'pokEm4HxuT5hGHmxe2ZOtO+IZo9Hvl/+qzRN5/mLCQEAHsLHzE6iBvELKCAZYUGrbNANf6xZSkfua2G17EDGo7zb'
        b'NGr5pjli3y+FvIERS0cW3BhAQkRiiA07ZEQ0IHh4Dku1re4sJcrwpo5UbFhZBMLSfJ0wjV4Z3rOwrf7RYh4t+erh'
        b'0IqrCWnXXj0xrORwbfnLzu7+och/4VeGtaazd64DqxvZQTuc5qMYxFsnGuQMnaVPgnQLlCq3Dvdnx2hjk9OcbHHX'
        b'EpvlL/ER5f4FyJsAm4Bdm2cnL1VJggrf7eLjmraWfSK9ya1ZFVnPDEu0wPr5xtsNbabOR+3K8HWnTi5QE5ML+MzR'
        b'zv97u9/+eQiJUAGKChvguymzFADsDd4AgqpS8VJUp1sheUyim8IreQBxaXjDnV28Ow275S+qPbGoFqwM2DTz6L6P'
        b'JdJKVanhEC7zxM6UAtrfQbgBmrRQNcXOXvm+Nemam+JyfmV44KzjIA7nUd22rD/3CzxMFCm4bjugiuqqOWBcn4y3'
        b'CG61Q/ixA3FWIWrfKpaBDgxHxQrzBubucBCWv6xHFPlpv8jwpb3HQgmNqGX5Uk7zjQroCz4eITPlmsALcmhBJNIU'
        b'VXq2il3AC696J5Y1rjz8L3Nwxd/8yof/4uTgiq9cOevMk2OVjqaHzGPJUeR45CgH/fKrhzWYDR8An/asza3FJgmI'
        b'Xb0BS+5B4WuWRaGWnayx6ZItgA/KCdtT6ydx261roa9aGd5/TsHnCLkulPxtH2X7CMnaYCBOX0ATVecLfwt0bGIB'
        b'BQAAgwrWIf41m9hGGV+0QeTOCgjT8t2S38eC8b0F6fOoKGGeOovVqxlrtOprUas8zlrlXK7O1vvR1lupM+3t76ar'
        b'xzPLrqftzd6I/v18EM3hMJeNp7G8hCDROGWt87kj7iiVNDvygGfy9j0oGIAnSK7kEnMWIcMPAevCvIWtv2Oq0lPH'
        b'UXMH3B/Pnp16IHOcf05EyScCDx7ZbI8e/gIQ0NLu1mOrPW1ucSTT0QSeQ9O5DyBVIT91DluKpbAI1PfE5ylKOGdC'
        b'zj4TviW2VKz4R9ibBMsixkeHygcXF+v5nNnwiUvTczTg6l651q6nReo++r0AE4gTio2dSdgAraFghEaCKK8Q0EVd'
        b'I9GoGXdWFklEV9gRSLsGIrtY3efOhu+9aHW3WsJGnI9nW73Wtm6MmnN/Hqp8qOnffQXiJDZbAkRqFRyI9SuJ3UN8'
        b'lrHHJGSFMtiBlTejQUawdFWBBzvIuw3GjKO0Fmj6vNnw7Rem6Xg3WtJ2ZYNdzoCbm/JonsVcv/ufwbpabDm4aVXS'
        b'S0DVynZB+Dj2tROm1qilbAVoVtXcQKD4sl8wJXdZWea8WL/nz4bvvFj9AG+Ox29e3drZ+eDNG3dY8HeuwFf7SNwF'
        b'59KAdRhsRQAvKatm+44ErINNaEhmjcs2RcuI6zMgp04uiFN89QWz4dsuXMMtmGyB/f7Ms2A/WVQeB3ZoGxvrZloA'
        b'r9Jg6d50AGPEb3yyMhW+64tPbAEJPtbAu5Q1YbF2L5x9bsTrPDvxkGkddsBaTVubae9wKx7ty9vs+ZkXYkcq3cBV'
        b'klDGcGgMUjAIQGu5BSehMXtNGd68NainO8CNkdWrnpIXSDaneOyLZsPHL0njsdxl/7GjsMORk3dGn5dhd6YaABAl'
        b'AIvzoJReZCAWWLlnT3TZ5l1PgGuihrZVKgsgkLQCCOHkrlOs++LZ8OcucXferuEqW5xtj1NNU4Vs9eb1+6/Xg/Uf'
        b'fR7gG4Jp4dPbxHOBeUdfaOPYsa8Dv3VsVpA9ZVrkITsSr82OPe17A83ObbHKX3iRAffIk+cT+W7stq3N62Oj/tW5'
        b'4Uf9vv5B8K+uJOgqwqjkYwCjpdJsu1pkUBxOkpyzRbpx9wYOt4mhyzQWuoZYxlvwBfp9ER+5LxXq7e1sAbieSCTk'
        b'Enk+vI8TPkelrv4xVhF70G5YqxtSMwEe6VLLIfhqwdfAHGQ1wrMdM6IQqJGubt6tlPWX47iHBUq9ZHGMXZpSh6fX'
        b'bXcsQLtdu01o5xvwjQS1s8iDTgg2sMM/CZ7ZXGIT/c5OwC1ZvvgzEQImFb02EnZDrM2Ltfvi2fDY8g+3bvPFsYf9'
        b'6q35ryf98ScQTq3OsrEply1ggRoWggYGIKa7DGQgx6Fi7FJeDeif1lG1KLHzvHbZB31KiHkpW78t1XRH5OnGY4gr'
        b'26sPzY13zDnWH+ZbZXZNksHig6uQvTjpVUIQKd6obkLK4OWGjRyTA1itshn4YTGIrDAv1nSxPl/Cx76XpM84evQf'
        b'v4KPlktXtbADFBAYjxA8kp+JmSfZHOaVu3cZu6xFJ+F48A+ZXUKqQPZz6pRs96V8TX456tya+XqdHe0Sw17nkbyO'
        b'QrPtee2S3dyV9ro1cIvmxpIZ0NuKWGkkaJQGjSoeTLEvVullbIxxeSqNRnrwlUzTHKcAE3DmOHv4qphdQNZm4/dU'
        b'Onh70sJZBHeWbwI/9ygTjJZ0l2Yc0bNAoz9+mUZyJ4xUhXbasKy82FQUsm+oYKragL+pgKhjMkAzHwEow44lWTEQ'
        b'sZ9XDWD0drFKf+IyjeRuGUkaJ7XgrN4sO6gaKI2TfKEDBAHmyp6cFbsJkY4NQq3vdZxGjEBX2ZTpFCNduZDEewgl'
        b'HnGrbAG+mtnw9Iiij9H7bwH+xypDcCzL7QzcCHguQRPlC78ijbSSHaIbkEPJxiN8I+pJQqjUTS96sVIvn51a7HW2'
        b'8qO5SvOOqYfOts6WcVVic+QA4qk5WbkpLzPLilIq1eGLScMUltfzCNkCwaLkyM5MHuAvmFPgw5exscHFaMEOsKt7'
        b'N69fT8eE7KPsK+Nsd2DJMRdlQMMKY5tsLPQGE2NdQpbA8BxyCW5t4zhFoDs4QFJAfqfQzVfMFtYhLD62np/WHZ5a'
        b'H+Px1bRbrnFi1/ys7lfeMk7eCUgZBrjaWuESkBkH6wDWAVmDDAe292WpBHt5IRA0Djgy0XcdQ4rBaLdI8n2eQ/7J'
        b'z9dzyC+/hHPIQ0A3Dns/vNA/YlL95taRpp/+GlrRmyaEZLMZ9l3zYMbGRt1LZPN7Pq9xuXfEdulC59unWJSICO5R'
        b'Q2Xg+MWavnI2HFwiZTxUdD7h/nTe+LoXgIJAFeulAE71raQewQjxb1QH4KfHLlUiNsQMXVqvGYsQ+FSqmS6SZ6XN'
        b'Yr1f9Tke85wfpENdJq+92+eM//Cn2OMJuK5xtKnWxmdj+LzUldIjHz7K2Gr0SnJ2dDAcjcpjWATPQPieUzsFpd/3'
        b'OR7TnUe7wz/hs25u7a+2D93cfCRtje8h5xp+50+xbShie4ktmj42P4yQHRAwj01CPSsXU43BKpsULIws7WNsIldf'
        b'OfMunpKe/9RsuLlkwIEERmPlm5tb9c7D4xNbcj1+IduJlgrsxL6hYPwML8AUFQG14xfO9qw9+ZjYN1/xhYRh94/K'
        b'MUalAdqfkhBePVvyu8mjf2WDR/67dXxstf7SLx/WlOh8Vptg+wbBLKdM5l6RKcBJjHIclCkBzVuLMTc4oYjJ6MAT'
        b'5ZgBpxbr8BXLaxp84v3bpCJbL2NlcPTYOpHDwADxvGd3CB2rRY5nqyHOuM+qpSgEe2m3XNkfEuwx5yilOQUvvYbd'
        b'h5bqZXPZ5wGQWh3un0PXukEmhagevPCd4YFPPJQrYOdAEc5l4BmDf7BySldkNSFol+CsbVLHjiBRT2FSq4vR35K0'
        b'mQPzH3oaYxvAQ/CeVfzZVCORjb0M1RSYyUpoFV3hOJvKUq4OJFjZyAuBLbMC+pSMdf9s6BdONQ7tUdhMtBotOrBd'
        b'wnob4GvPqnIZJR9us0bBsFIrK45OywAg2Ccp6Sqk7LracsrZ5dXZwoeJS9RkNIl4Bw9RZIcxJOmQsoqHzsUKZaMT'
        b'jsMhhQ6OrSo1WHtQzRdA3Fg6nK4hJ9hTFBGXSWj9CUIrgqbvBFYFNh68Qk7XR3hgPaeusDsOx9N20wwwInJKDrFz'
        b'DGXIRp6ikrxMQutvEdqIuFtdYy/zInwxnFwUwdVDMMUZIFlhgQNA+rBpYqzOhqoTyKB3wD0AQYs1UrPhLYuYxo0d'
        b'prrxZa7lOfeNtDuO8Lwl/RESP7plfDbfYSK+2sL6SyO0HPN6AN1z1sYuPQmG8kh5VQQPDm6ARl12QSgdRC7ulFMs'
        b'vZhXnKvyvO/ufFvbXt08fHnMgVJXxeob3vKmt7/7XSPm/ErJJ/kulhak5eOSWKvnlb9vuufxlB9fMdInB27JV/hC'
        b'JXBJvuYNNF1rpxySmNnwXZelllwl+vzwfCTVBnNx39qZHxB/6X1ImVYhG3pkdl1AngAFAEU1T4Kr6qEkfK93tsZy'
        b'vC1uUBImDl6zL6/04hRgZtn8dULJ411zVHkw7Wf4Ym2Hy3C17pQRIyBHNpJbrNR8qIYUN3Y2t/c3bmyl7VGnj7wE'
        b'KacWL7FBXAA+sbYocACpm1ack6ZFaKWUbGvgWXFVpeZmBDspJCUTWOEpMMAtOAtajk6Ov9/Hp26Ua618cNRsVOqv'
        b'ryD7JIf4LJXmxDzRHWidBgVSWaWQBfvydhd04pUTWxzJrlrmC1jQQjaKPQVo+gUIenlKXU+b25vbD23k3bRdrgHU'
        b'tZvz+5ePPJdNsxIvOYNzwaVgg/R8TQcGmAsftSKVmmwR11Pz2cFElY3HRREBqK15fwrMCRyatSTFRm9rH27l5v6t'
        b'CW8bN3Z3rt+Y2+ihF0EVBYzgkW9qEUCeHlnfGDZY5IAkX4FFcy7BAjxUjuHssUiFyG7H1meinxII42zYvayI4Vff'
        b'9eBb3/a2UauPPZ/z9YCWI9lNymwL2ZF5WCDkI7iMF7xj4TyLHARSkIodygMEVSFa0b1Zt1irr5wNL19ED04E439N'
        b'sR2EcxxVUgC52C4nddGSK5bvth0SpOoduCXzJQegpuMEKFiA46M955+esqe/agGMnJf/HbsIk+LelKP4u3fAwoj7'
        b'sVeDCpcYOufvBiRrjd9keADCkyopSnBtO7ZI5AOmLovn4PEeHMtOQVm6tqccZXz1AvRFTdTRxScln9DCnTXgvufF'
        b'4xUqX/w0Fvwj9MiidNEuSV6KgBMXwSNgxFlEYxnZmRuZFGwF7B4xWp3m969dEHCXqdLuze07A+7HnzKswa8bZ28m'
        b'jmrghGzsah+tzoIjgDhKg56uKjZDx/bWIrH43ghQBNfmk7QXKPW6BZlxKUotTCI32duvJxB6lTnq0XMOiEnAwbWl'
        b'Nk66R3QA72985aFAz5psySBASYYn5BZ1Cqh8/WzYv1CdFuaQL3ouxzInUBodwf2V5WxZNkeIubIDJkBNZot6y7HN'
        b'km8eVI0sI4KOIosoej3FAddOiQ5PUK/HTSH/8AWIDtFUtl7pwNQ8F4PFIDZgfwJuUXzDldmKhk1W2R0zew4iliYZ'
        b'QG3oeEo1whtOmWl6f3poPIfZ++Dm1tbePH6ttt43yyaZCv/W1VtJ4Z8+n/QEG6HbqCXwMBYUiawEl72u44QgW33k'
        b'GB1gSsEGW8Gp2Hg+HRuQF+RfLOfXzIYfWGI8/lxU22297VKQO7ABz5bGBPRUuFi3nhOyODW2IG45Xx0yasrOZzYZ'
        b'04D+oXMAVYkC/CZhGyHisTm8w4qcEg7euIDkMLEfX4B8bonnzvh2x0nZxjFOGNX6AOyIcOUBTbRCbtWpVNvBY22J'
        b'pjltMl8R8OQ5YEd5wEgWpwPeSN86dg9wczv1WuhrP1+vhd70JClPf/OTqzz9LU+a8vSv+zwvT3/r5315+gOf1+Xp'
        b'D86Gj17SveXRdV/Z2tmbR5zjCsr3wEeBFsHolQBEQHCVFVET0Ex0VmE7C6hTNZ9tg/Y4bLnInWkQaJNpBn4bzCkJ'
        b'cv2rT+09caLL5q0nZ7ttH0C4Qdxrm1v1qCJ5LK0HHHrk6B39M78ABEZxsg47onaJ1JBrlsm1FlzCZyVSL7AAZEG2'
        b'U/QFP8lMWUDVxuobFSeeyr12ZXjXExR4XvXPBd65uX/j5v4qEvfRIt+S9zkvGWtV2Su0t8ixJ4GDs9if2HEWlbXI'
        b'Xxn0SuJjWcuAeO9KtE0rsjHlABQm5H3dyvQJ0FzeRzdh7rY9Cn3U0EqxfdiHx+CMnxx7LIxLLHWEtFfne2Cz4Ovj'
        b'uf1ewV891OG33jJODLG2WSdUzwV5p3Ekd0qaA7GzDx3UPoMYI2+J6gw8t7O8q7jog2qpTDzyW3/9yvC+sz1PPMqd'
        b'xq0S7B85y23lB//oTcNaJvarpeREGk/EwOP6yJEs0qaitQJCmrf7BaAy3oP0JqMrQC5rbtuE0Gsrw3vPJvThG9D9'
        b'tr16eDh1q7r5UOanPhPOnYtsLFHikW7oLvIJCNhf5ZNbNshL1sDzfeaoGa+tFsU2eBQ0FE34PiHzG0517s9xoQ/F'
        b'v+0y901PQSJIID9sAY1MALflQEXBagDXAa2l8C07w8lYfKDr4S+KbbhDQKhJQgMETMj7NWzYtrxO7qAX+PGNzW1s'
        b'0o1r2LA7vY8d3Dd3EemDsU2l6BXycbFWlMhp8Nh8YNnjOQEvpPm8QVk4kscKqZq6SwnQFIhzohvrG1fArJfbiH6h'
        b'Cnv3grFmFUIAR1a9lJq1sqrUkrQwzoaxT1ATvA4AoxUcJmxhsKSsB4dVseopFb52ZfiGJaqwtTkfd/PTPKlhtU+V'
        b'JkI6kLEWVDKOHf75+p2N80AzQWWCTYnn4AghuSULAoC/A3GnmhG/aXmd2ynt3NM3Ht3c3walHAV//nuZL03xI/0H'
        b'5Ika+MchIWoTwbsy9mJqY/evQP6cEV66ZUQUUmYXkDqn2g+/mQ+0lyd4bVvpsVY3rgGJt9255GvXOQfN9x5ls+BI'
        b'WiOQJKc1z/Id/D7XJCG/r6EFxO1kOlJnE9rr1Dmp2ukpyd+yMnxgiZJvbu+33UfS1sYRFKPov/kVSDumSaNFHeeC'
        b'ufFRh1TJgx2wxzY7O2eAbCcR0rsEvZbBeo6gVB0517Wp5ttftzJ88xJF52LfJvZ73gZfKaIl7EtQOVUdq1da4aM/'
        b'XgMBtZSKdYWHdPb9CFrHxvOA0gsHumkfpxq2v3W5Tn4YUU4KPvsWrLdju1Wje5cCmBa/VWwa1YECM/YsQIvK2MBA'
        b'DYjgyo5D6DJ8X1vez00K/sCFRPPd9mjarRv48lade/pbf3JYSxA2c20NvLdnlUOtXbWOzVkR223WSKnI80T2HmEG'
        b'sLcbx8biobNHxIT4Dy7XXQ7lZpOeudifAKCNwZoYDaJHQeJno2zJO13H4dyej+lUqpXNvLxIzrCcEuxDax98TNZP'
        b'jlV4G3P+Mld9/9pc3p9/IYseAUCaQfAoJinJbnyG4x7g015lKKFqaQgmwZdWcwRYdwqUQUdlI9Di1ISp9ZWlD4k8'
        b'9JSSyrV26ChXj8HOqMsfvmBYq8r4xGnDrEHF5mR/jWaL7EbaCPQFDGB7UVxvVQTbiLvO+Z4em6KKONWO/e3LdZm0'
        b'd6Ntb9zY2dos8436kdcDt9QeCQcVJ9nV1sc+LQTafIwJfuEAE2PpDJuyKYjORKtz8IiZgJRTQ23fsdyNelLsjTlC'
        b'n3vQHle9BMRGuEyVoCZKAfWWkIHB3NicLanEt7SQN2mExFq6IACLnOvSfYtTHvT1y8Msb0y7D+3cv79zfetg/ZNP'
        b'4TBkkOUqQBli54RgIStSPUGKMNiOwI/FBUm6xj6gbHHi8A3HHv6wT56aYfENy5aW72wOHpDvZDF99QJE3rnIuSxV'
        b'2W47R+kmC35ZZeQjV5li4KBTBd0ShPVKmipTjpyRMyHvO0+Rl7+/WnZZAn789cO38aMnlJ3twhL4DXxrv109Cazu'
        b'fR+iXzQpulJdQGaElZPHDoRbFF4gq+ABtw0WHDEdArNHqZYc3QINkDGtnfKFd7HF11Kl3eBAq3GE+e6MS2zH0QCu'
        b'26J5Yqu79k3nqliKV5AIkRUVvBsYRKvoEVCE8dnqmjzvVMQUpno3+fByhc43t+vWKPb691xlngHwSBKsUehSZGIP'
        b'8M7zdF+Ewvoil0B0kDb4C4hBIycOiUcnBijMT42r+sYLEHp8gD4K7SUIcbU1Y+9jXVPTrJoPdn6Pgb0XkGRa4V1V'
        b'CgCIwNlZ8FW9UgUMqIJpTjnze0jilyP0jWuP7fHUZGN7p84X+gXfDe+wFVzLSRX4DktGjmCr0QXsS85wiJl3GAU4'
        b'NbBrJTi8AhaXOYsKcuTtFIx678rwjiXJzKvCMp/4fAVOkbxJsoOFASbF1rPrvQqEEKEaAsR4fw5SU1iwEUQFF9MZ'
        b'ECrzIRUcfCqTvO+U+TZTsnJSStveu7k3Sjtel2yUawk5/JCL7bUxaV8lbJqz4Hn4uO8VCM6lisYX+s1K71qL4yhO'
        b'oxR+l+DnoLiWLTZB1o0wI+1lxzBsYB+QWKbk/6YnGOxul/9G7vsjwNsAo+FNwSHceMDeM75Xq9bBDyRLdCXvWDk2'
        b'CzZvLlusuKzIe/ARxO2AaI0w3fmgTSaeySYdpiLHN68Mb38i8u4/dqPtHXoGXOHq7taNk6s+Lu3PvhGi6qIzjz6y'
        b'EhY7ynH8QWbPAA80YWPsTkWGEQ58tLyF0uD0ziNsABFOjqr5lpXhm5YIMjia72Q++Ugc1hx4V4982ix6DsjOANMd'
        b'INWz3Wgah3wrmZIy7EKqMqgaR28islTb67y+5U6p//RyodFtAxJPiv+p2bDWADLACpP34IJQBdjC8DhSRze+60Ua'
        b'CY4phK3RwRZcTmAv+Cofh/g8Jf77lzcwa2I04kkFfosl67LlEkPjCUjN7MFgvLXdO/aXZKthBGjndc4BmwAB0oFH'
        b'mIzkCDxdDgdm3KHAB1aG1501+B0dPn3sOYjLBMmsKJXFsFINwdjCORCcBc1fsb+aBwwFgST95RsU7QD4WTCVnJ5y'
        b'6I2VYX1Jcfn6Th0lffszhzWtY2ZvgdAEHyx0TmVoYt5XprEDoDQcw8jWJI3T323pnHvowFrgMZNDfL91ZXjtWSWt'
        b'8zXc+jRLkmrivBDOmUvIDc5yhgW2Um9ZA7hnrVjb4LGCnqMvizFZGrZ9YMuEySOCtDK84Rxr2DcfGqUL2Pwp84bC'
        b'd0hQiMMaz+csKyWL5xyiJERhiWoClfAKyRe7qgYgOfA/VyYzb14Z3nxW6e6Mqk//JUTVGDn6RsBUHBzXi4ZFRVek'
        b'FNp4LYMDmnQuGemwXVyKwJkRX40AOlZNJayyMrzxrCJeb/uw6P58OPxbKwKQ0Q3BMtjxjXcVMgBQhSLAdoRLlbiA'
        b'RXaZBVoRsDzXyjnk7JuMOJqnUGJlV9DlokTkrFHiB7FZlDMgkVmzCROvSoUGXGmpwiXBf7PWgDTYy0i6bFkkGpAX'
        b't1XjmAYf/NSKtpXBnirxI2277uxe3QXt2qv5xCZ54N3XRw7mRNQJ2I4v5r2VgB5VNhcz562EkCKr+aQyrbDuXTbZ'
        b'AMUTDI4YRfY+IVN/vA08KdPRBNX5Qfar2IZcRctJ07ArT/WwKxCUm/HdcxQ72/8ilFhApRCyZclCQ4yWrnak9BCn'
        b'VuuhleH1T1Syo8GubBMxyvbbz0NwwZaE1STgj9OgK5INCTyrVV1GtBt7L7VxrmXn0zpnQy6BN6Z8ASHEVPK79ni5'
        b'Y3rVNvcbfHJndxTtB989rAWtQY9bDZxm0GFPEThzNrCiQwNwOgcAmmMA+Km+6txjCjaze3gU+EtThwCbZzPozo2x'
        b'nn10toNPz0Y2ajgsy/GxvM082G8WCyR7I9nnYKqOL4jINwZ8b9UA4+j/rPlrZpIjPbyyuKryFNlubG5v3yoCHRev'
        b'Ph3pzEsTvcjsMmO6V70IX7M5nHATEtJdcuCbxZWoAWoqA5/12DDsA6KnTjg/uDKsPVEBy87WzevbGz1d39yaM8w3'
        b'vRSWtTBSYO89lg8Xh4XCDpHJJIRlzqRBNqm+cLCCAhwIGquXALaCrwgzbepKamtl+KonKlzvmxs39ze3Rrle/mXc'
        b'qECtTfBfRKJiU8CgOahLG2wOD3uDDZTEwr/SC/7fAxKdtSC/obLv3IRc15d7b0P4N/ab35j3mx9F/4OXgLTHpomy'
        b'syimSgOqYKKpXsbCN4scOwna0IRtmcdQEQRTmsyZLfiVB2kTom+z5OGCMPhdOnwSQFzlyKregB0VutGc98oZ33zx'
        b'HmGQxMI7fDGwpSGiVEgpQVWNeASbuDQFwXZWhs2LA+J3afGbQOOIVoAZRTiprOUhX+4CNKKWzOGSDRZAmiwaGEi1'
        b'xgZjTPHJAkMBkCunJ7S4ccpV1DlO1zb4vnoe1V7+G+MZW68s32eneWM4ZV1JGfl6Cj7CBnzeC25d1tYFZZsFl5Ys'
        b'S+nYoMmoKf//0MrwqlNFP3n0++p7GfVj5NQn07NkKxnpOCiUs3kaZ1kpHQAdAcH5EMp41ozxzEdVjve2h1OH7pBh'
        b'93OTYX6g6z7KlTCcytYYEjnjUXtdoe04gEoD9sjavPDRkoUD+2g2guI7oeICEbo3UyuxtzK8ZmHV/EOb++wTtMdC'
        b'7MO+RwcP/ghbfffmHXsfBqeR/aA2YHMEaQYAZG9TUAHOVmFXqugMFwk4UbBFewd/nbpf21/eLOSjiDS+R7hrM/yl'
        b'l3HeXW/I4zAkIATnesFWohcJhBaE6PD7oqXVXQsZYzNVSwNq7Svgb0Oqmor0N1eGrYsKS9OK/BhikwwwbfG+JPBs'
        b'C0NnbGy4hGL7KctiWs1egcC3nTOEiapMAj/zWjTf6lR8fWRl2L642DStym+zG0RDoAmREyeAASJSns4VqS029pFP'
        b'MiTkig4cCtfX7G6he+d0U57WtFqnIN+jyz3voPzj2Ju7FXjtK7E5U+w9QjpVjS2WTSl9Y2szE7Lma71kWeIMK7GS'
        b'y+dKuxhwkCCDRyacUODDK8P1i3KqBZr8ZXoVFrxmbAhbJFIeAHUzTVZslxAatg0ZNSvUNT0OMYjkGhAdXpasLX0q'
        b'zDy2/HvdE161QJfPwq2c941vpiB/RSRyBnDEp94bIhHAezCQXILaWjBGhHJAJwlOi59HvuHF6oQu37aytKYvR241'
        b'L9W9W4OnvnpYgxUC4IfsHKlt5j2TS0TKU10bLx1YbdQhdERgzXJXz+KuzIc61bO7woQG/9zy9/ixXy1S5SdY9GUy'
        b'SC5AFJBUbtABrFwYyZ6nKYGVE4nI2BM2f2AlbBH4bkAGIXRBvplQ5dtXhhsX51iLlPk9tv3jOzCWfKUCmoe19w7U'
        b'JijkY1kyZ3Fk2Cj1WsYmUMCLCvvdK5s481VMIarvWH7AGmcD363Az6zysSRcPyJU1SZFAbNRSAvCdQ7WyQQQwfGJ'
        b'm+GUqJx4kVILUj0Hp4KzpSkFvvMCA9YCTRiwbEhY4+jB1RRcJrYC5weuxY4xKnSr2S63RKuNTd7ApQKft+LnRNIc'
        b'FzWhyUcuNGAt0GUesKriFbEyHugS1I6z6qUSIHK+Ixk69uJBrOIAzMijhyw5/wbfs4DEcSqNfHT5bjUOQb5bgW+X'
        b'w1rXCKMFllBOuYhNDkanc1MQE4IjWyOlCMumpAp8n83SWDFZdQKGzUArEwp81wW61QJN6FYcNcq2DIixNoXAbBFZ'
        b'2By7rzoYFtSyetKwYUPrmR0Le/dGyMITvzx1QvbdF+pWC3ShW3V2AkPOG1/FSRC7BMyugwc+h6RWsh9pB4FViLMw'
        b'D1iO4GR5BY1TmUaKf2Zlugn/vJ6ctZwP3f6gMxw+1pHyqAvp5nUOTgj2zG44dkA9oesDn/pVHoQ1xNrG1MjeQgAn'
        b'jZE3ag3N2RAVfAnYuLLFUOngK4gI2GJ8YiNdYKORCW0/tnzL3fLB+Rjruy33s/DCbISQlrfW43Ej3xgDQHYdsusN'
        b'uR+bDXDfwrygAKKycQ1yEo+HtAHcn7pH+56V4UMX6IWLtHkxtbEVO4RMECE7Jmx8XmD2CABAfKyy47sYxA6heFYE'
        b'AmA6B6WzmteFMAXzv/cC0cx8nvXdqvyNGc8Lm+SQXll5d88OdbKOrTqNdpz/kC1iBxvXAGSyXKwIQAOQGcHX+m7y'
        b'AP37LhTNLFLm99n4l20rkhYVdDJzrFgPAXnHIr/00ivvPnMEw0nVCNdc0gZYzls3coAgp+zy8QuM2uNg67s1+fEZ'
        b'G/JlkTlYhf2OgpalqAj47HJlwMZWEY1VcNIh8AHfVAuD8Al/6/C8bKb2/vdfaNReoMvvjsdOiL58NtNSBrZnwSlH'
        b'GlYOYNSFN6lZN1iqZjU2HYkg9UlanUIpxbYpq/zzF8kp20KriKgd0oDRIlf2DEg8KgKDcbwQLq4CswXezLpiQkRa'
        b'ymmclmiLFz4U3SY0+cTFcsq20CoWSVRlQ0qf4F9RRd7tiSrxpQ6+L5vUogAhmyaqsmKsNYFmCF9ZBjkFN4flnwgf'
        b'W+XoMenG+Ax71OJXZpzNwD6UwTuwdg0k2Xjoq/RYy++UAbE3kSUFFQiNIToBzmG7W24jB8owocUPrAwfvDiLTOjx'
        b'LHoW33NmwEwpgubLypoy4itJcinGJaN5BsMKaMMygCh6rjw+xs9ZBLCpq/4/uzJcu3Br5Kjml+2/TKhZuoscf9wA'
        b'UqJsuquk4C0QP8PZEl/59Zg7u7yWYDyyf0CWARYo7J0/FbT+3Mrw8CUY41iNZ8zG56EiRezz4GJugJNJZGWyKVhz'
        b'jpeRACrJNttgLC9K0d1HDo0UURSoN+VTP3gZtmgg8se2YKpOuQbX2WlCWANBrQ++5IQtX9nvLHHpYRletSVsCGcM'
        b'O/B4zTL8CSV+6HJscazGM8aIi/QGKZNHPGWbKmOd5+xQDrKvXvL6Gli3svpbN6dsEAlK+4bvsJBjQo0fvgxbIOqE'
        b'Y1tY6VpjS2m+1YkWuR3E0So+kbZFRJ8AqCqShYECzeqaI6cqJ06BVqximFDik5dji2M1aIviA1I5Yo4XKbDDXDe6'
        b'aBCyWjs7oclYwPJt4Xxr55yUVoJEAlq2sYp1CsMfXIYtkKXVsS2gkDWgS6Tw3OPsmul77aBXvH9rRPMKhCoDuDf2'
        b'B+9s1gLixVYWfbIW7UcuxxbHatAWvUsDsMRbaGtziDl2tlhQFuBWeI04ldjOVYEuVqsBFsEhmwJ+ZEhOYep84l+4'
        b'DFv0nPWxLbztbuyqiPwQqlMZSLzzHRCUcpLBC+Q+6RK6Z1Ury3xggg712DJ8siT3z1+OLY7VGPNFAJjwYHgyGsfa'
        b'xuz4niIrwFrZowDxk2C/ukt4mlO82VTWBvBbpWpUU7b4Fy/DFh5Lf2wLh4BUJPZpVK0ykTmAWGUdNrjIHjkjWtGM'
        b'8AYkkCdiEtgwe05bUjH2ybvhH70cWxyrQVvIxD4CpmSO4NINLo9tA0zFbqtgHH08gkw+yF4McGMQfM0QpdIdmBA0'
        b'fkKNf+lScjdc59a+4LlcKrUY3iTwjt6l7grbZuqWC/ZHAxKsiskx87k5cganyAP7Arm7KcL0Fy4pdx+pMeZurXJh'
        b'qUNWgcUmpvnkTSBgbTID33KLs6o2WI/0KNhBRLG4CtxQBns3HOSM8/v/f+zdfRg='
    )),
}

def _actual_wire_fixture_part(name):
    size, sha, encoded = _ACTUAL_WIRE_FIXTURE[name]
    raw = zlib.decompress(base64.b64decode(encoded))
    if len(raw) != size or hashlib.sha256(raw).hexdigest() != sha:
        raise AssertionError('complete actual regression fixture identity')
    return pickle.loads(raw)


class ActualWireLifetimeTests(unittest.TestCase):
    def test_actual_complete_graph_fits_after_last_permit_use(self):
        # Complete real compiled/extra graph and caller bodies, including their
        # within-part aliases. Separate immutable aliases earn no charge credit.
        compiled, extra = _actual_wire_fixture_part('compiled')
        case = _actual_wire_fixture_part('caller')
        roots = case['roots']
        roots['permits'] = _actual_wire_fixture_part('permits')
        self.assertEqual(fixed.core.SEMANTIC, 2097152)
        self.assertEqual(len(compiled['maxima']), 81)
        self.assertEqual(len(json.loads(compiled['prior83_wire'])), 83)
        before = case['intrinsic_charge'] + case['scratch'] + fixed.graph_charge(
            tuple(roots[name] for name in case['order']))
        with mock.patch.object(fixed.core, 'canonical', side_effect=AssertionError('early encoding')):
            with self.assertRaisesRegex(ValueError, 'wire preallocation lifetime'):
                fixed.invocation_wire_manifest(compiled, extra, live_bytes=before)
        # The actual driver reads/authenticates/debits this final source pin
        # before this transfer. These three roots have no subsequent consumer.
        for name in ('permits', 'permit_pin', 'bootstrap_journal'):
            del roots[name]
        roots['final_source_pin'] = case['source_snapshot_pin']
        after = case['intrinsic_charge'] + case['scratch'] + fixed.graph_charge(
            tuple(roots[name] for name in case['order'] if name in roots)
            + (roots['final_source_pin'],))
        self.assertGreater(before - after, 190000)
        with mock.patch.object(fixed, '_read', side_effect=AssertionError('late input read')):
            ids, wires, setup = fixed.invocation_wire_manifest(compiled, extra, live_bytes=after)
        self.assertEqual(len(ids), 676)
        self.assertEqual(len(wires), len(ids))
        self.assertEqual(wires[:setup], compiled['setup_unit_wire'])
        self.assertEqual(wires[setup:setup+len(compiled['units'])], compiled['units'])
        self.assertIs(wires[setup], compiled['units'][0])
        for wire, rows in zip(wires[setup+len(compiled['units']):], extra):
            self.assertEqual(json.loads(wire), json.loads(fixed.core.canonical(rows)))
        # Closing reads must not weaken the unchanged allocation guard.
        with mock.patch.object(fixed.core, 'canonical', side_effect=AssertionError('early encoding')):
            with self.assertRaisesRegex(ValueError, 'wire preallocation lifetime'):
                fixed.invocation_wire_manifest(compiled, extra,
                    live_bytes=fixed.core.SEMANTIC-fixed.graph_charge((compiled, extra)))


class OriginalProfileTest(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
  self.path=Path(self.tmp.name)/'profile.json';self.debits=[]
  anchor={'path':str(Path(self.tmp.name)/'authority.json'),'bytes':2,'sha256':hashlib.sha256(b'{}').hexdigest()}
  profile={name:None for name in runtime.RuntimeAdmission.PROFILE_FIELDS}
  profile.update(schema='fixed40-development-runtime-profile-v1',authority=anchor,history=anchor,events=[214,0,0],source_identity='a'*64)
  self.raw=fixed.core.canonical(profile);self.path.write_bytes(self.raw)
  self.pin={'path':str(self.path),'bytes':len(self.raw),'sha256':hashlib.sha256(self.raw).hexdigest()}
  self.a=object.__new__(runtime.RuntimeAdmission)
  self.a.profile_pin=dict(self.pin);self.a._graphs=[];self.a._outer_live=0;self.a.read_trace=[]
  self.a._full_read_charge=lambda pins:self.debits.extend(pins)
  self.a._profile_wire=fixed.core.canonical({'development':{},'authority':anchor,'history':anchor,'events':[214,0,0],'consumed':{},'controller':'retained-v1'})
  self.live=fixed.graph_charge((self.a._profile_wire,self.a.profile_pin,self.a.read_trace))+32768
  self.a.caller_charge=self.live
 def test_authenticates_original_not_migration_wire(self):
  result=fixed._compiled_profile_binding(self.a,live_bytes=self.live)
  self.assertNotIn('source_identity',json.loads(self.a._profile_wire))
  self.assertEqual(result['source_identity'],'a'*64)
  self.assertEqual(result['profile'],self.pin)
  self.assertEqual(self.debits,[self.pin])
  self.assertEqual(self.a.read_trace[-1]['bytes'],len(self.raw))
  self.assertTrue(self.a.read_trace[-1]['eof'])
 def test_full_hash_failure_remains_charged(self):
  self.path.write_bytes(self.raw[:-1]+b' ')
  with self.assertRaisesRegex(ValueError,'full hash/EOF'):
   fixed._compiled_profile_binding(self.a,live_bytes=self.live)
  self.assertEqual(self.debits,[self.pin]);self.assertEqual(self.a.read_trace,[])
 def test_near_cap_rejects_before_read_or_decode(self):
  with patch.object(fixed.records,'decode',side_effect=AssertionError('decode reached')):
   with self.assertRaisesRegex(ValueError,'predecode lifetime'):
    fixed._compiled_profile_binding(self.a,live_bytes=fixed.core.SEMANTIC-1)
  self.assertEqual(self.debits,[])
 def test_active_constructor_graphs_are_not_discounted(self):
  self.a._graphs=[{'retained':True}]
  with self.assertRaisesRegex(ValueError,'completed profile admission'):
   fixed._compiled_profile_binding(self.a,live_bytes=self.live)
  self.assertEqual(self.debits,[])
 def test_compiler_carries_new_trace_graph_to_next_guard(self):
  self.a.check_compilation=lambda *args:None
  before=self.a.caller_charge;external=17000;observed=[]
  projected={'profile':dict(self.pin),'authority':json.loads(self.raw)['authority'],
   'history':json.loads(self.raw)['history'],'events':[214,0,0],'source_identity':'a'*64}
  def frontier(*,live_bytes):
   observed.append(live_bytes)
   self.assertGreater(self.a.caller_charge,before)
   self.assertEqual(live_bytes,external+self.a.caller_charge+fixed.graph_charge(projected))
   raise StopIteration('next actual compiler frontier')
  with patch.object(fixed,'selected_descriptor',side_effect=frontier):
   with self.assertRaisesRegex(StopIteration,'next actual compiler frontier'):
    fixed.compile_selected40(consumed_events=(214,0,0),event_history=fixed.BASELINE_HISTORY,
     admission=self.a,live_bytes=external)
  self.assertEqual(len(observed),1);self.assertEqual(self.debits,[self.pin])


class Round152QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=77.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 78.0, 'baseline_used_percent': 22.0, 'consume_percent': 10.0, 'floor_remaining_percent': 68.0, 'observed_utc': '2026-10-24T11:35:32.039000+00:00', 'resets_at': 1793382070, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '9f3d646a2d506f7aeb79347c2d1849251ba8393996a16ffe291cc518ef427475')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=10.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=68.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(68.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((68.0, 0), (67.0, 0), (77.0, -11), (77.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_twenty_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_twenty_points':
                value['consume_percent'] = 20.0
            if case == 'rollback':
                value.update(remaining_percent=79.0, used_percent=21.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'78.0', b'77.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)


class Round153QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=71.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 72.0, 'baseline_used_percent': 28.0, 'consume_percent': 10.0, 'floor_remaining_percent': 62.0, 'observed_utc': '2026-10-24T15:28:22.856000+00:00', 'resets_at': 1793382070, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '43757136fef817299f46151d78b9059cb4fd45388476b72874363a7715705c08')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=10.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=62.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(62.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((62.0, 0), (61.0, 0), (71.0, -11), (71.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_twenty_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_twenty_points':
                value['consume_percent'] = 20.0
            if case == 'rollback':
                value.update(remaining_percent=73.0, used_percent=27.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'72.0', b'71.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)


class ActualRole32HookLifetimeTests(unittest.TestCase):
    """Exact failed MODE inputs and pinned reads; never real hook acceptance."""

    @classmethod
    def setUpClass(cls):
        import ast
        import pa1_fixed40_constructors as c
        cls.c=c
        cls.root=Path('/home/fry/artifacts/vrf40-gen17-20261024-round153')
        cls.directory=cls.root/'complete166-generation-v1'
        cls.planned_path=cls.directory/'planned-normal-commit.json'
        cls.planned=cls.planned_path.read_bytes()
        cls.control=json.loads(cls.planned)
        cls.log_pin=cls.control['log']
        cls.log=Path(cls.log_pin['path']).read_bytes()
        cls.planned_pin=dict(path=str(cls.planned_path),bytes=len(cls.planned),sha256=hashlib.sha256(cls.planned).hexdigest())
        index=json.loads((cls.root/'source-image-166.json').read_bytes())
        prior=next(x for x in index['files'] if Path(x['path']).name=='pa1_fixed40_constructors.py')
        source=Path(prior['snapshot']).read_bytes()
        assert hashlib.sha256(source).hexdigest()==prior['sha256']
        tree=ast.parse(source)
        fn=next(x for x in tree.body if isinstance(x,ast.FunctionDef) and x.name=='prospective_normal_hook_summary')
        env=dict(c.__dict__)
        exec(compile(ast.fix_missing_locations(ast.Module(body=[fn],type_ignores=[])),prior['snapshot'],'exec'),env)
        cls.old_function=staticmethod(env['prospective_normal_hook_summary'])

    def reader(self,planned_pin,log_pin):
        import os
        import stat
        from types import SimpleNamespace
        c=self.c
        class PinnedReader:
            """Test-only actual full file reader with the GEN Reader limits."""
            def __init__(self):
                self.declarations=tuple((i,c.fixed._pin_tuple(p),'evidence','bytes') for i,p in enumerate((planned_pin,log_pin)))
                self.wire=c.core.canonical(self.declarations)
                self.declaration_charge=c.fixed.graph_charge(self.declarations)
                name='materialize-native-role32'
                metadata=12288+128+128+len(self.wire)+256+len(name)+256+1+128+32768
                self.invocation=SimpleNamespace(operations=bytearray(2048),metadata_charge=lambda:metadata)
                self.used=set();self.reads=[]
            def consume(self,occurrence,pin,*,kind,live_bytes):
                row=self.declarations[occurrence]
                if occurrence in self.used or row[1]!=c.fixed._pin_tuple(pin) or kind!='evidence':
                    raise ValueError('test exact declared occurrence')
                if self.invocation.metadata_charge()+self.declaration_charge+live_bytes+c.core.CHUNK+262144>c.core.SEMANTIC:
                    raise ValueError('test complete reader lifetime')
                self.reads.append(pin['bytes']+1)
                fd=os.open(pin['path'],os.O_RDONLY|os.O_NOFOLLOW)
                try:
                    before=os.fstat(fd)
                    if not stat.S_ISREG(before.st_mode) or before.st_size!=pin['bytes']:raise ValueError('test pinned regular size')
                    digest=hashlib.sha256();total=0
                    while True:
                        block=os.read(fd,c.core.CHUNK)
                        if not block:break
                        digest.update(block);total+=len(block)
                    after=os.fstat(fd)
                    if (before.st_dev,before.st_ino,before.st_size,before.st_mtime_ns)!=(after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns):raise ValueError('test changed pinned body')
                    if total!=pin['bytes'] or digest.hexdigest()!=pin['sha256']:raise ValueError('test full pinned SHA/EOF')
                finally:os.close(fd)
                self.used.add(occurrence)
        return PinnedReader()

    def call(self,function=None,*,caller=600000,planned=None,planned_pin=None,log=None,log_pin=None):
        planned=self.planned if planned is None else planned
        planned_pin=self.planned_pin if planned_pin is None else planned_pin
        log=self.log if log is None else log
        log_pin=self.log_pin if log_pin is None else log_pin
        unit=self.reader(planned_pin,log_pin)
        function=self.c.prospective_normal_hook_summary if function is None else function
        result=function(maxima=self.c.ROLE_MAXIMA,planned_raw=planned,planned_pin=planned_pin,full_log=log,unit=unit,declarations=unit.declarations,live_bytes=caller)
        return result,unit

    def test_exact_real_bodies_fit_with_large_caller_and_full_reads(self):
        with self.assertRaisesRegex(ValueError,'predecode and scan lifetime'):
            self.call(self.old_function)
        raw,unit=self.call()
        value=json.loads(raw)
        self.assertEqual(value['schema'],self.c.PROSPECTIVE_MODE)
        self.assertEqual(value['exit_code'],2147483647)
        self.assertEqual(value['matched_tests'],2147483647)
        self.assertEqual(unit.used,{0,1})
        self.assertEqual(unit.reads,[len(self.planned)+1,len(self.log)+1])
        self.assertEqual(self.c.core.SEMANTIC,2097152)

    def test_full_log_binding_and_allocation_rejection_remain(self):
        with self.assertRaisesRegex(ValueError,'complete hook log binding'):
            self.call(log=self.log[:-1]+bytes([self.log[-1]^1]))
        with self.assertRaisesRegex(ValueError,'predecode and scan lifetime'):
            self.call(caller=2097152)

    def test_passing_test_claim_at_tail_is_still_rejected(self):
        directory=self.root/'role32-negative-fixture'
        directory.mkdir(exist_ok=True)
        line=b'\ntest result: ok. 1 passed; 0 failed; 0 ignored\n'
        log=self.log[:-len(line)]+line
        log_path=directory/'NONHISTORICAL-log.txt'
        # Reuse only exact immutable NONHISTORICAL fixture bytes; never overwrite.
        if log_path.exists():
            self.assertEqual(log_path.read_bytes(),log)
        else:
            with log_path.open('xb') as f:f.write(log)
        log_pin=dict(path=str(log_path),bytes=len(log),sha256=hashlib.sha256(log).hexdigest())
        control=dict(self.control,log=log_pin)
        planned=(json.dumps(control,sort_keys=True,separators=(',',':'))+'\n').encode()
        planned_path=directory/'NONHISTORICAL-control.json'
        # Reuse only exact immutable NONHISTORICAL fixture bytes; never overwrite.
        if planned_path.exists():
            self.assertEqual(planned_path.read_bytes(),planned)
        else:
            with planned_path.open('xb') as f:f.write(planned)
        planned_pin=dict(path=str(planned_path),bytes=len(planned),sha256=hashlib.sha256(planned).hexdigest())
        with self.assertRaisesRegex(ValueError,'cannot claim passing tests'):
            self.call(planned=planned,planned_pin=planned_pin,log=log,log_pin=log_pin)



class Round154QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=67.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 68.0, 'baseline_used_percent': 32.0, 'consume_percent': 10.0, 'floor_remaining_percent': 58.0, 'observed_utc': '2026-10-24T17:27:23.922000+00:00', 'resets_at': 1793382070, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '267fc4ba27c7c985f82628a34990e2997bcf18bcae0dfe31cc05d7d08e1d7587')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=10.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=58.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(58.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((58.0, 0), (57.0, 0), (67.0, -11), (67.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_twenty_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_twenty_points':
                value['consume_percent'] = 20.0
            if case == 'rollback':
                value.update(remaining_percent=69.0, used_percent=31.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'68.0', b'67.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)

class ExactHistorical577AliasTests(unittest.TestCase):
    """Derived four-row fixture tests the exact reviewed alias, not execution."""
    def fixture(self):
        value = json.loads(fixed._read(fixed.RETAINED_CHILD_ALIAS_PIN))
        rows = [[r['occurrence'], list(fixed._pin_tuple(r['logical'])), r['kind'], r['mode']]
                for r in value['rows']]
        return value, dict(units=(fixed.core.canonical(rows),), logical_aliases=())

    def test_exact_four_rows_preserve_logical_bodies_and_order(self):
        value, plan = self.fixture()
        result = fixed.retained_child_wires(plan, live_bytes=600000)
        rows = json.loads(result['units'][0])
        self.assertEqual([r[0] for r in rows], [571,577,578,579])
        self.assertEqual(len(result['logical_aliases']), 4)
        for original, row, alias in zip(value['rows'], rows, result['logical_aliases']):
            self.assertEqual(row, [original['occurrence'], list(fixed._pin_tuple(original['physical'])), original['kind'], original['mode']])
            self.assertEqual(alias['logical'], original['logical'])
            self.assertEqual(original['logical']['bytes'], original['physical']['bytes'])
            self.assertEqual(original['logical']['sha256'], original['physical']['sha256'])

    def test_missing_retargeted_and_duplicate_occurrences_rejected(self):
        for kind in ('missing', 'retarget', 'duplicate'):
            value, plan = self.fixture()
            rows = json.loads(plan['units'][0])
            if kind == 'missing': rows = [r for r in rows if r[0] != 577]
            if kind == 'retarget': rows[1][1][0] += '.wrong-original'
            if kind == 'duplicate': rows.append(rows[1])
            plan['units'] = (fixed.core.canonical(rows),)
            with self.assertRaises(ValueError):
                fixed.retained_child_wires(plan, live_bytes=0)

    def test_existing_alias_collision_and_allocation_stay_strict(self):
        value, plan = self.fixture()
        plan['logical_aliases'] = (dict(occurrence=577),)
        with self.assertRaises(ValueError):
            fixed.retained_child_wires(plan, live_bytes=0)
        value, plan = self.fixture()
        with self.assertRaises(ValueError):
            fixed.retained_child_wires(plan, live_bytes=2097152)


class ActualColdAssemblyOwnerTests(unittest.TestCase):
    """Use full real prior83/profile/input bodies; never claim runtime fit."""
    @classmethod
    def setUpClass(cls):
        import ast
        import pa1_fixed40_constructors as constructors
        cls.artifacts=Path('/home/fry/artifacts/vrf40-alias577-20261024-round154')
        old_index=json.loads((cls.artifacts/'source-image-169.json').read_bytes())
        old_pin=next(r for r in old_index['files'] if Path(r['path']).name=='pa1_fixed40_constructors.py')
        old_source=Path(old_pin['snapshot']).read_bytes()
        if hashlib.sha256(old_source).hexdigest()!=old_pin['sha256']:raise AssertionError('exact frozen old29source')
        old_fn=next(n for n in ast.parse(old_source).body if isinstance(n,ast.FunctionDef) and n.name=='prospective_validation_summary')
        old_env=dict(constructors.__dict__)
        exec(compile(ast.Module(body=[old_fn],type_ignores=[]),old_pin['snapshot'],'exec'),old_env)
        cls.old_role29_function=staticmethod(old_env['prospective_validation_summary'])
        source=(cls.artifacts/'generate_complete81_v20.py').read_bytes()
        tree=ast.parse(source)
        functions=[n for n in tree.body if isinstance(n,ast.FunctionDef)
                   and n.name in ('freeze_assembly_owners','reload_assembly_owners')]
        if len(functions)!=2:raise AssertionError('exact cold-owner source functions')
        namespace={}
        exec(compile(ast.Module(body=functions,type_ignores=[]),
                     str(cls.artifacts/'generate_complete81_v20.py'),'exec'),namespace)
        cls.freeze=staticmethod(namespace['freeze_assembly_owners'])
        cls.reload=staticmethod(namespace['reload_assembly_owners'])
        historical=fixed.historical_plan_wires(live_bytes=0)
        prior=fixed.prior83_bindings(historical['units'],live_bytes=fixed.graph_charge(historical))
        prior_wire=fixed.core.canonical(prior)
        if len(prior)!=83:raise AssertionError('full83 real prior bindings')
        arguments=json.loads((cls.artifacts/'complete169-generation-v1/arguments-29.json').read_bytes())
        profile_pin=arguments['byte_inputs']['current_profile_raw']
        profile=json.loads(fixed._read(profile_pin))
        binding=dict(profile=dict(profile_pin),**{k:profile[k] for k in ('authority','history','events','source_identity')})
        cls.compiled=dict(maxima=constructors.ROLE_MAXIMA,prior83_wire=prior_wire,
            consumed_events=(214,0,0),inherited_event_history=fixed.BASELINE_HISTORY,
            profile_binding=binding)
        cls.inputs=json.loads(Path('/home/fry/artifacts/vrf40-admission-01a14b66-20261018/semantic-inputs-v5.json').read_bytes())
        cls.inputs['shared_reports']=tuple(cls.inputs['shared_reports'])
        cls.inputs['native_reports']=tuple(cls.inputs['native_reports'])
        for key,name in (('source_freeze','current90-source-freeze.json'),('tool_freeze','current11-tool-freeze.json')):
            path=cls.artifacts/'current169-namespace-v1'/name
            body=path.read_bytes()
            cls.inputs[key]=dict(path=str(path),bytes=len(body),sha256=hashlib.sha256(body).hexdigest())

    @classmethod
    def tearDownClass(cls):
        cls.compiled=None;cls.inputs=None

    def fixture(self):
        from contextlib import contextmanager
        root=Path(tempfile.mkdtemp(prefix='cold-owner-regression-',dir=str(self.artifacts)))
        owners=[]
        @contextmanager
        def own(value):
            owners.append(value)
            try:yield
            finally:self.assertIs(owners.pop(),value)
        def encode(value,maximum):
            raw=fixed.core.canonical(value)
            self.assertLessEqual(len(raw),maximum)
            return raw
        def publish(name,body):
            self.assertTrue(owners)
            path=root/name
            with path.open('xb') as f:f.write(body)
            return dict(path=str(path),bytes=len(body),sha256=hashlib.sha256(body).hexdigest())
        pins=self.freeze(self.compiled,self.inputs,publish=publish,encode=encode,own=own)
        self.assertEqual(owners,[])
        def load_body(pin):
            # Both decoded envelopes must remain owned during the prior83 read.
            self.assertEqual(len(owners),2)
            return bytes(fixed._read(pin))
        def load_json(pin):return json.loads(fixed._read(pin))
        return pins,owners,own,load_json,load_body,root

    def test_complete_real_graph_roundtrip_preserves_bytes_types_and_events(self):
        pins,owners,own,load_json,load_body,root=self.fixture()
        compiled,inputs=self.reload(*pins,load_json=load_json,load_body=load_body,own=own)
        self.assertEqual(compiled,self.compiled)
        self.assertEqual(inputs,self.inputs)
        self.assertIs(type(compiled['maxima']),tuple)
        self.assertIs(type(compiled['consumed_events']),tuple)
        self.assertIs(type(inputs['shared_reports']),tuple)
        self.assertIs(type(inputs['native_reports']),tuple)
        self.assertEqual(compiled['prior83_wire'],self.compiled['prior83_wire'])
        self.assertEqual(owners,[])
        self.assertLess(fixed.graph_charge(pins),fixed.graph_charge((self.compiled,self.inputs)))
        self.assertTrue(all(set(p)=={'path','bytes','sha256'} for p in pins))

    def test_corrupt_original_body_is_rejected_and_all_owners_unwind(self):
        pins,owners,own,load_json,load_body,root=self.fixture()
        prior=dict(pins[1]);body=Path(prior['path']).read_bytes()
        path=root/'NONHISTORICAL-corrupt-prior83.json'
        with path.open('xb') as f:f.write(bytes([body[0]^1])+body[1:])
        prior['path']=str(path)
        with self.assertRaises(ValueError):
            self.reload(pins[0],prior,pins[2],load_json=load_json,load_body=load_body,own=own)
        self.assertEqual(owners,[])

    def test_unknown_compiled_field_is_rejected_before_publication(self):
        from contextlib import nullcontext
        calls=[]
        bad=dict(self.compiled,omitted_owner_claim=True)
        with self.assertRaises(ValueError):
            self.freeze(bad,self.inputs,publish=lambda *x:calls.append(x),
                        encode=lambda *x:b'',own=lambda value:nullcontext())
        self.assertEqual(calls,[])

    def test_actual_role29_prefix_release_preserves_all143_full_reads(self):
        """Replay frozen logical inputs; reconstructed roots are not history."""
        import ast
        import os
        import stat
        import time
        import pa1_fixed40_constructors as c
        root=self.artifacts/'complete169-generation-v1'
        program=(self.artifacts/'generate_complete81_v20.py').read_text()
        tree=ast.parse(program)
        failure=json.loads((root/'generation-FAILED.json').read_bytes())
        arguments=json.loads((root/'arguments-29.json').read_bytes())
        config=json.loads((self.artifacts/'generation-config169-v2-LAUNCH.json').read_bytes())
        def pin(path):
            body=path.read_bytes()
            return dict(path=str(path),bytes=len(body),sha256=hashlib.sha256(body).hexdigest())
        recipe_pins={}
        def recipe(role):
            path=root/'recipes'/('%02d.json'%role)
            raw=path.read_bytes()
            recipe_pins[role]=dict(path=str(path),bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
            return json.loads(raw)
        dep=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='dependencies')
        main=next(n for n in tree.body if isinstance(n,ast.Try) and any(
            isinstance(v,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='dependency_map' for t in v.targets) for v in n.body))
        start=next(i for i,n in enumerate(main.body) if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='dependency_map' for t in n.targets))
        stop=next(i for i,n in enumerate(main.body) if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='ORDER' for t in n.targets))
        env=dict(recipe=recipe,fixed=fixed,C=c)
        exec(compile(ast.Module(body=[dep]+main.body[start:stop+1],type_ignores=[]),'frozen-GEN19-dependencies','exec'),env)
        order=env['ORDER'];remaining=env['dependency_map']
        completed=[v['role'] for v in failure['cases']]
        self.assertEqual(list(order[:len(completed)]),completed)
        self.assertEqual(order[len(completed)],29)
        for role in completed:remaining.pop(role)
        pending_dependencies=remaining.pop(29)
        cases=failure['cases']
        models={int(k):v for k,v in failure['model_pins'].items()}
        controls={int(k):v for k,v in failure['argument_controls'].items()}
        for case in cases:
            case['arguments']=controls[case['role']]
            case['output']=models[case['role']]
        review_controls={int(path.name.split('-')[1]):pin(path)
                         for path in root.glob('review-*-NOT-EXECUTED.txt')}
        phase_terms=[{k:value[k] for k in ('role','required')}
                     for value in (json.loads(line) for line in (root/'complete-constructor-lifetimes.jsonl').read_text().splitlines())]
        namespace=self.artifacts/'current169-namespace-v1'
        prefix=dict(CONFIG=config,OUT=root,PLAN=dict(path='/home/fry/artifacts/vrf40-complete-fit-20261019/concrete-argument-program-v1/all81-concrete-argument-recipes.json',bytes=881537,sha256='08bd2e4223ab707d59b870aa630dc8191678b804c7fa5bde263501673f1ac193'),
            models=models,controls=controls,completed=completed,cases=cases,
            recipe_wires=recipe_pins,review_controls=review_controls,
            fact_seal_control=pin(root/'fact-seal-inputs-NOT-EXECUTED.json') if (root/'fact-seal-inputs-NOT-EXECUTED.json').exists() else None,
            phase_terms=phase_terms,current_index_pin=config['current_source_index'],
            current_profile_pin=arguments['byte_inputs']['current_profile_raw'],
            current_profile=None,current_sources=None,legacy=None,
            current_archive_pin=pin(namespace/'exact-archive-manifest.json'),
            current_budget_pin=config['saved_budget'],current_quota_pin=config['current_quota'],
            prior_bundle_pin=recipe(42)['parameters']['prior_bundle'],
            prior_report_pin=recipe(42)['parameters']['prior_report_pin'],
            schedule_template=config['schedule_template'],
            native_identities={name:json.loads((root/('native-'+name+'-identity.json')).read_bytes())['pin'] for name in ('live','replay')},
            planned_commit=pin(root/'planned-normal-commit.json'),dependency_map=remaining,ORDER=order,
            compiled_assembly=self.compiled,current_inputs=self.inputs,
            current_projection_pin=pin(root/'current-reference-placement-projection.json'),
            current_scope_pin=pin(namespace/'historical-gate-scope.json'),helper_roots=[])
        names=ast.literal_eval(next(n.value for n in tree.body if isinstance(n,ast.Assign)
            and any(isinstance(t,ast.Name) and t.id=='GENERATOR_ROOT_NAMES' for t in n.targets)))
        control=controls[29]
        pending=dict(role=29,arguments=control,output=None,
                     constructor_rows=pin(root/'constructor-rows-29.json'),dependencies=pending_dependencies)
        kwargs=dict(arguments['parameters'])
        for name,value in arguments['byte_inputs'].items():kwargs[name]=bytes(fixed._read(value))
        for name,value in arguments['json_inputs'].items():kwargs[name]=json.loads(fixed._read(value))
        for name,value in arguments['wire_inputs'].items():kwargs[name]={k:bytes(fixed._read(v)) for k,v in value.items()}
        original_kwargs_count=len(kwargs)
        declared=json.loads((root/'constructor-rows-29.json').read_bytes())
        rows=tuple((i,fixed._pin_tuple(p),kind,mode) for i,(p,kind,mode) in enumerate(declared))
        self.assertEqual(len(rows),143)
        old_index=json.loads((self.artifacts/'source-image-169.json').read_bytes())
        # This diagnostic replays old169 logical sources from their exact saved
        # bytes when current test/C files changed. It grants no runtime alias.
        physical={r['path']:dict(path=r['snapshot'],bytes=r['bytes'],sha256=r['sha256'])
                  for r in old_index['files'] if Path(r['path']).name in ('pa1_fixed40_constructors.py','test_pa1_fixed40.py')}
        declared=None;old_index=None;arguments=None;failure=None
        declarations_expected=sum(row[1][1]+1 for row in rows)
        dependencies=[n for n in tree.body if isinstance(n,(ast.FunctionDef,ast.ClassDef))
                      and n.name in ('Reader','stream_full','check_deadline')]
        stream_directory=Path(tempfile.mkdtemp(prefix='role29-prefix-stream-',dir=str(self.artifacts)))
        stream_journal=(stream_directory/'full-read-journal.jsonl').open('x')
        self.addCleanup(stream_journal.close)
        stream_env=dict(journal=stream_journal,fixed=fixed,core=fixed.core,records=fixed.records,
            os=os,stat=stat,time=time,json=json,hashlib=hashlib,SimpleNamespace=SimpleNamespace,
            CONFIG=dict(deadline_seconds=300),started=time.monotonic())
        def canonical(value,maximum):
            raw=fixed.core.canonical(value)
            self.assertLessEqual(len(raw),maximum)
            return raw
        stream_env['canonical_owned']=canonical
        debits=[]
        def charge(p,kind):
            debits.append((p['bytes']+1,kind))
            stream_journal.write(json.dumps(dict(event='BEFORE_OPEN',pin=p,bytes_plus_EOF=p['bytes']+1,kind=kind,scope='FROZEN_REPLAY_NO_RUNTIME_ALIAS_AUTHORITY'))+'\n')
            stream_journal.flush();os.fsync(stream_journal.fileno())
        stream_env['charge']=charge
        exec(compile(ast.Module(body=dependencies,type_ignores=[]),'frozen-GEN19-reader','exec'),stream_env)
        full_stream=stream_env['stream_full']
        def stream(p,**options):
            target=physical.get(p['path'],p)
            self.assertEqual((target['bytes'],target['sha256']),(p['bytes'],p['sha256']))
            return full_stream(target,**options)
        stream_env['stream_full']=stream
        Reader=stream_env['Reader']
        call_number=[0]
        def call(extra=0,function=None):
            debits.clear()
            external=fixed.graph_charge(tuple(prefix[n] for n in names if n in prefix))+32768
            external+=fixed.graph_charge((control,pending))+512+256*original_kwargs_count
            external+=fixed.graph_charge(physical)+extra
            unit=Reader(29,rows,external,control)
            measurement=dict(schema='NONHISTORICAL-actual-regression-caller-v1',external=external,invocation_metadata=unit.invocation.metadata_charge(),declarations=unit.declaration_charge,owner=external+unit.invocation.metadata_charge()+unit.declaration_charge,historical_actual_owner='UNKNOWN',scope='Constructor/full143read proof only; declaration encoder is width-only fixture; all production GEN guards remain required')
            with (stream_directory/('caller-%02d.json'%call_number[0])).open('x') as f:json.dump(measurement,f,sort_keys=True)
            call_number[0]+=1
            raw=(c.prospective_validation_summary if function is None else function)(**kwargs,maxima=c.ROLE_MAXIMA,
                live_bytes=external,unit=unit,declarations=unit.declarations)
            self.assertEqual(unit.used,set(range(143)))
            self.assertEqual(len(debits),143)
            self.assertEqual(sum(n for n,_ in debits),declarations_expected)
            self.assertEqual(json.loads(raw)['schema'],c.PROSPECTIVE_MODE)
            return external
        old_external=fixed.graph_charge(tuple(prefix[n] for n in names if n in prefix))+32768
        with self.assertRaisesRegex(ValueError,'deferred profile predecode lifetime'):
            call(function=self.old_role29_function)
        self.assertEqual(debits,[])
        pins,owners,own,load_json,load_body,fixture_root=self.fixture()
        # Fresh prospective fixture from the exact corrected producer expression.
        # Original169failed controls remain immutable and are never relabelled.
        producer_fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='materialize')
        source_expression=next(n.value for n in ast.walk(producer_fn) if isinstance(n,ast.keyword) and n.arg=='source' and isinstance(n.value,ast.Call) and 'current_sources' in ast.unparse(n.value))
        corrected_arguments=json.loads((root/'arguments-29.json').read_bytes())
        old_review=corrected_arguments['byte_inputs']['deferred_review_raw']
        deferred=json.loads(kwargs['deferred_review_raw'])
        deferred['context_derivation']['known']['source']=eval(compile(ast.Expression(body=source_expression),'exact-GEN20-context-expression','eval'),dict(core=fixed.core,current_sources=kwargs['source_bindings']))
        replacement_raw=fixed.core.canonical(deferred)
        self.assertLessEqual(len(replacement_raw),65536)
        replacement_path=fixture_root/'NONHISTORICAL-corrected-deferred-review.json'
        with replacement_path.open('xb') as f:f.write(replacement_raw)
        replacement=pin(replacement_path)
        kwargs['deferred_review_raw']=replacement_raw
        kwargs['complete_offline_fit_review']=replacement
        corrected_arguments['parameters']['complete_offline_fit_review']=replacement
        corrected_arguments['byte_inputs']['deferred_review_raw']=replacement
        corrected_path=fixture_root/'NONHISTORICAL-corrected-arguments-29.json'
        with corrected_path.open('xb') as f:f.write(fixed.core.canonical(corrected_arguments))
        control=pin(corrected_path);prefix['controls'][29]=control;pending['arguments']=control
        old_tuple=fixed._pin_tuple(old_review)
        self.assertEqual(sum(r[1]==old_tuple for r in rows),2)
        rows=tuple((i,fixed._pin_tuple(replacement) if physical_pin==old_tuple else physical_pin,kind,mode) for i,physical_pin,kind,mode in rows)
        corrected_rows=fixture_root/'NONHISTORICAL-corrected-constructor-rows-29.json'
        with corrected_rows.open('xb') as f:f.write(fixed.core.canonical([[fixed._pin(p),k,m] for _,p,k,m in rows]))
        pending['constructor_rows']=pin(corrected_rows)
        declarations_expected=sum(r[1][1]+1 for r in rows)
        deferred=corrected_arguments=None
        prefix['cold_assembly_pin'],prefix['cold_prior83_pin'],prefix['cold_inputs_pin']=pins
        prefix['compiled_assembly']=None;prefix['current_inputs']=None
        self.__class__.compiled=None;self.__class__.inputs=None
        try:
            new_external=call()
            self.assertLess(new_external,old_external)
            with self.assertRaisesRegex(ValueError,'deferred profile predecode lifetime'):
                call(2097152)
            self.assertEqual(debits,[])
            report=dict(schema='NONHISTORICAL-frozen-prefix-regression-v1',
                complete_named_roots=sorted(prefix),old_root_charge=old_external,
                released_call_external=new_external,full_declared_reads=143,
                read_bytes_plus_EOF=declarations_expected,source_snapshot_replay=physical,
                historical_actual_owner='UNKNOWN',no_runtime_fit_or_native_execution=True)
            with (fixture_root/'actual143-regression-result.json').open('x') as f:json.dump(report,f,sort_keys=True)
        finally:
            self.__class__.compiled,self.__class__.inputs=self.reload(*pins,
                load_json=load_json,load_body=load_body,own=own)


class Round155QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=59.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 60.0, 'baseline_used_percent': 40.0, 'consume_percent': 10.0, 'floor_remaining_percent': 50.0, 'observed_utc': '2026-10-24T20:38:33.336000+00:00', 'resets_at': 1793382070, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '18eb6f30a422db55b146307ca2fab45bf901532edd6be3ee92589c5b90403773')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=10.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=50.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(50.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((50.0, 0), (49.0, 0), (59.0, -11), (59.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_twenty_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_twenty_points':
                value['consume_percent'] = 20.0
            if case == 'rollback':
                value.update(remaining_percent=61.0, used_percent=39.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'60.0', b'59.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)


class ActualRole42ControlLifetimeTests(unittest.TestCase):
    """Complete frozen caller/control regression; never a full GEN or fit PASS."""

    @classmethod
    def setUpClass(cls):
        import importlib.util
        cls.root = Path('/home/fry/artifacts/vrf40-role42-20261024-round156')
        spec = importlib.util.spec_from_file_location('role42_frozen_fixture', cls.root/'measure_role42.py')
        cls.fixture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.fixture)
        cls.source = cls.root/'generate_complete81_v21.py'

    @classmethod
    def tearDownClass(cls):
        cls.fixture.journal.close()

    def test_complete_real_control_preserved_with_one_parameter_owner(self):
        old = self.fixture.measure(Path('/home/fry/artifacts/vrf40-alias577-20261024-round154/generate_complete81_v20.py'))
        new = self.fixture.measure(self.source)
        self.assertFalse(old['passed'])
        self.assertEqual(old['error'], 'complete control serialization caller lifetime')
        self.assertEqual(old['required'], 2516913)
        self.assertTrue(new['passed'])
        self.assertEqual(new['required'], 1843752)
        self.assertEqual(new['serialized_bytes'], 88454)
        self.assertEqual(new['control_sha256'], '8c5f0040711977f2ca7d6fc96c6588aab29f859461c4ec6cc7cc3edca9ed2c3c')
        self.assertEqual(old['width'], new['width'])
        self.assertEqual(old['global_charge'], new['global_charge'])
        # Execute the exact post-publication transfer, preserving every value.
        import ast
        tree = ast.parse(self.fixture.read(self.source))
        fn = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'materialize')
        publish = next(i for i,n in enumerate(fn.body) if isinstance(n, ast.Assign)
                       and any(isinstance(t,ast.Name) and t.id == 'control' for t in n.targets)
                       and isinstance(n.value, ast.Call) and isinstance(n.value.func, ast.Name)
                       and n.value.func.id == 'put')
        transfer = fn.body[publish+1]
        self.assertIsInstance(transfer, ast.If)
        parameters = {'full_fields': [1, {'preserved': True}]}
        env = dict(role=42, kwargs={}, resolved_control={'parameters':parameters})
        exec(compile(ast.Module(body=[transfer],type_ignores=[]),str(self.source),'exec'),env)
        self.assertIs(env['kwargs'], parameters)
        self.assertIsNone(env['resolved_control'])

    def test_exact_allocation_boundary_rejects_before_encoding(self):
        slack = 2097152 - 1843752
        exact = self.fixture.measure(self.source, extra_caller=slack)
        self.assertTrue(exact['passed'])
        self.assertEqual(exact['required'], 2097152)
        rejected = self.fixture.measure(self.source, extra_caller=slack+1)
        self.assertFalse(rejected['passed'])
        self.assertEqual(rejected['required'], 2097153)
        self.assertEqual(rejected['error'], 'complete control serialization caller lifetime')
        self.assertNotIn('control_sha256', rejected)
        self.assertNotIn('serialized_bytes', rejected)


class Round156QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=55.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 56.0, 'baseline_used_percent': 44.0, 'consume_percent': 10.0, 'floor_remaining_percent': 46.0, 'observed_utc': '2026-10-24T22:05:51.748000+00:00', 'resets_at': 1793382070, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '13865f51521f56a3f4e121f063614bad057ebf9f947ca1e6c529a90396d8f31e')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=10.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=46.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(46.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((46.0, 0), (45.0, 0), (55.0, -11), (55.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_twenty_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_twenty_points':
                value['consume_percent'] = 20.0
            if case == 'rollback':
                value.update(remaining_percent=57.0, used_percent=39.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'56.0', b'55.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)


class ActualNativeColdCaseTests(unittest.TestCase):
    """Real full role54 constructor and four cold-owner paths; no runtime fit."""
    @classmethod
    def setUpClass(cls):
        import importlib.util
        path=Path('/home/fry/artifacts/vrf40-native54-20261025-round157/native54_fixture_v3.py')
        spec=importlib.util.spec_from_file_location('native54_frozen_fixture',path)
        cls.module=importlib.util.module_from_spec(spec);spec.loader.exec_module(cls.module)
        cls.base=cls.module.Native54Fixture()

    @classmethod
    def tearDownClass(cls):
        cls.base.close()

    def fixture(self):
        value=self.base.fork();self.addCleanup(value.close);return value

    def test_actual_full_native54_and_negative_guard(self):
        f=self.fixture()
        with self.assertRaisesRegex(ValueError,'complete input preallocation'):
            f.call(maps=False)
        self.assertEqual(f.native_calls[-1]['required'],2267168)
        self.assertEqual(f.native_calls[-1]['input_graph'],783998)
        self.assertEqual(f.last_reader.used,set())
        self.assertEqual(f.env['total'],0)
        raw=f.call(cold=True)
        self.assertLessEqual(f.native_calls[-1]['required'],2097152)
        self.assertEqual(f.last_reader.used,set(range(70)))
        self.assertEqual(f.full_declared,8481867)
        value=json.loads(raw)
        self.assertEqual(value['schema'],self.module.C.PROSPECTIVE_MODE)
        self.assertFalse(any(value['qualification'].values()))
        self.assertEqual(f.env['cases'],f.original_cases)
        self.assertNotIn('cold_completed_cases_pin',f.env)
        self.assertNotIn('cold_native_maps',f.env)

    def test_all_four_roles_restore_complete_identity_order_and_bytes(self):
        for role in (54,69,72,75):
            with self.subTest(role=role):
                f=self.fixture();before=self.module.fixed.core.canonical(f.original_cases)
                f.env['freeze_completed_cases'](role)
                self.assertEqual(f.env['cases'],[])
                pin=f.env['cold_completed_cases_pin']
                self.assertEqual(f.read_pin(pin),before)
                f.env['restore_completed_cases']()
                self.assertEqual(self.module.fixed.core.canonical(f.env['cases']),before)
                self.assertEqual([r['role'] for r in f.env['cases']],f.env['completed'])
                self.assertEqual(f.env['total'],len(before)+1)
                self.assertNotIn('cold_completed_cases_pin',f.env)

    def test_tamper_missing_and_authenticated_reorder_reject(self):
        for kind in ('tamper','missing','order'):
            with self.subTest(kind=kind):
                f=self.fixture();f.env['freeze_completed_cases'](54)
                original=dict(f.env['cold_completed_cases_pin']);body=f.read_pin(original)
                target=f.out/(kind+'.json')
                if kind=='tamper':
                    with target.open('xb') as stream:stream.write(bytes([body[0]^1])+body[1:])
                elif kind=='order':
                    rows=json.loads(body);rows[0],rows[1]=rows[1],rows[0]
                    with target.open('xb') as stream:stream.write(self.module.fixed.core.canonical(rows))
                changed=dict(original,path=str(target))
                if kind=='order':changed=f.pin(target)
                f.env['cold_completed_cases_pin']=changed
                with self.assertRaises((ValueError,FileNotFoundError)):
                    f.env['restore_completed_cases']()
                self.assertEqual(f.env['cases'],[])
                self.assertIsNotNone(f.env['cold_completed_cases_pin'])
                self.assertEqual(f.read_pin(original),body)

    def test_pre_restore_exception_keeps_full_cold_reference(self):
        f=self.fixture();f.env['freeze_completed_cases'](54)
        original=dict(f.env['cold_completed_cases_pin'])
        try:
            f.call(extra=2097152)
        except ValueError as error:
            with self.assertRaises(ValueError):
                f.env['handle_failure'](error)
        else:
            self.fail('full native caller guard must reject before restoration')
        value=json.loads(f.read(f.out/'generation-FAILED.json'))
        self.assertEqual(value['schema'],'fixed40-complete81-generation-result-v2')
        self.assertEqual(value['retained_completed_cases'],original)
        self.assertEqual(set(value['retained_native_maps']),{'models','controls','recipe_wires'})
        for name,pin in value['retained_native_maps'].items():
            rows=json.loads(f.read_pin(pin))
            self.assertEqual([row[0] for row in rows],list(self.base.env[name]))
        self.assertEqual(value['case_order'],'retained-completed-prefix-then-hot-cases')
        self.assertEqual(value['cases'],[])
        self.assertEqual(value['completed_roles'],[r['role'] for r in f.original_cases])
        self.assertEqual(json.loads(f.read_pin(original)),f.original_cases)
        self.assertFalse(value['accepted']);self.assertFalse(value['ready'])

    def test_cold_native_exact_overflow_rejects_before_source_reads(self):
        f=self.fixture();f.env['freeze_completed_cases'](54)
        f.env['freeze_native_maps'](54)
        now=(f.env['generation_caller']()+self.module.fixed.graph_charge((f.control,f.pending))+512+256)
        expected=2267168+now-f.external_old
        extra=2097152-expected+1
        self.assertGreater(extra,0)
        with self.assertRaisesRegex(ValueError,'complete input preallocation'):
            f.call(extra=extra,maps=False)
        self.assertEqual(f.native_calls[-1]['required'],2097153)
        self.assertEqual(f.last_reader.used,set())
        self.assertEqual(f.env['total'],0)

    def test_case_only_baseline_rejects_larger_native_maxima(self):
        for role in (72,75):
            with self.subTest(role=role):
                f=self.fixture()
                with self.assertRaisesRegex(ValueError,'complete input preallocation'):
                    f.call(cold=True,maps=False,guard_role=role,guard_only=True)
                self.assertGreater(f.native_calls[-1]['required'],2097152)
                self.assertEqual(f.last_reader.used,set())
                self.assertEqual(f.env['total'],0)

    def test_full_typed_maps_and_larger_guard_maxima(self):
        for role in (69,72,75):
            with self.subTest(role=role):
                f=self.fixture()
                before={name:list(f.env[name].items()) for name in ('models','controls','recipe_wires')}
                result=f.call(cold=True,guard_role=role,guard_only=True)
                self.assertLessEqual(result['required'],2097152)
                self.assertEqual(f.last_reader.used,set())
                for name,rows in before.items():
                    self.assertEqual(list(f.env[name].items()),rows)
                    self.assertTrue(all(type(k) is int for k in f.env[name]))
                self.assertEqual(f.env['cases'],f.original_cases)
                self.assertNotIn('cold_native_maps',f.env)

    def test_native_map_authenticated_order_and_missing_body_reject(self):
        for kind in ('order','missing'):
            with self.subTest(kind=kind):
                f=self.fixture();f.env['freeze_completed_cases'](54);f.env['freeze_native_maps'](54)
                original=dict(f.env['cold_native_maps']['recipe_wires'])
                body=f.read_pin(original);target=f.out/(kind+'-map.json')
                if kind=='order':
                    rows=json.loads(body);rows[0],rows[1]=rows[1],rows[0]
                    with target.open('xb') as stream:stream.write(self.module.fixed.core.canonical(rows))
                    f.env['cold_native_maps']['recipe_wires']=f.pin(target)
                else:f.env['cold_native_maps']['recipe_wires']=dict(original,path=str(target))
                with self.assertRaises((ValueError,FileNotFoundError)):
                    f.env['restore_native_maps'](54)
                self.assertIsNotNone(f.env['cold_native_maps'])
                self.assertEqual(f.read_pin(original),body)

    def test_invalid_role_and_hot_order_reject_before_create(self):
        f=self.fixture()
        for role in (True,53,76):
            with self.assertRaisesRegex(ValueError,'closed native cold-case role'):
                f.env['freeze_completed_cases'](role)
        self.assertEqual(list(f.out.glob('cold-completed-cases-*.json')),[])
        f.env['cases']=list(reversed(f.original_cases))
        with self.assertRaisesRegex(ValueError,'hot case order'):
            f.env['freeze_completed_cases'](54)
        self.assertEqual(list(f.out.glob('cold-completed-cases-*.json')),[])


class Round157QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=51.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 52.0, 'baseline_used_percent': 48.0, 'consume_percent': 10.0, 'floor_remaining_percent': 42.0, 'observed_utc': '2026-10-25T00:19:46.022000+00:00', 'resets_at': 1793382070, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '20061e7a6d4a74cf59f25113b76f9779e116b5e3d7755dc13988319c5918de4b')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=10.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=42.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(42.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((42.0, 0), (41.0, 0), (51.0, -11), (51.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_twenty_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_twenty_points':
                value['consume_percent'] = 20.0
            if case == 'rollback':
                value.update(remaining_percent=53.0, used_percent=39.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'52.0', b'51.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)


class Round158QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=46.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 47.0, 'baseline_used_percent': 53.0, 'consume_percent': 10.0, 'floor_remaining_percent': 37.0, 'observed_utc': '2026-10-25T02:39:32.336000+00:00', 'resets_at': 1793382070, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, 'ebf3890edcdd2e8fd59e2d094722e8d309296b43d44e33ae5b40f863043844df')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=10.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=37.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(37.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((37.0, 0), (36.0, 0), (46.0, -11), (46.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_twenty_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_twenty_points':
                value['consume_percent'] = 20.0
            if case == 'rollback':
                value.update(remaining_percent=48.0, used_percent=39.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'47.0', b'46.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)


class Round159QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=99.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 100.0, 'baseline_used_percent': 0.0, 'consume_percent': 10.0, 'floor_remaining_percent': 90.0, 'observed_utc': '2026-10-25T11:38:03.903000+00:00', 'resets_at': 1793532099, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '54734d020bff9f39a417e8a96816c6ad7fc03207fc2905d4374e437bb98c4efd')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=10.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=90.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(90.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((90.0, 0), (89.0, 0), (99.0, -11), (99.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_twenty_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_twenty_points':
                value['consume_percent'] = 20.0
            if case == 'rollback':
                value.update(remaining_percent=101.0, used_percent=-1.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'100.0', b'99.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)


class Round160QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=96.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 97.0, 'baseline_used_percent': 3.0, 'consume_percent': 20.0, 'floor_remaining_percent': 77.0, 'observed_utc': '2026-10-25T13:03:28.110000+00:00', 'resets_at': 1793532099, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, 'eee18ba4a2aba8901e8ae6b14d2def895f79824df5c9d54696e183fdf7d2379b')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=20.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=77.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(77.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((77.0, 0), (76.0, 0), (96.0, -11), (96.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_ten_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_ten_points':
                value['consume_percent'] = 10.0
            if case == 'rollback':
                value.update(remaining_percent=98.0, used_percent=2.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'97.0', b'96.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)
        self.assertEqual(unittest.defaultTestLoader.loadTestsFromTestCase(Round160FullRole41MaterializeTests).countTestCases(), 5)


def _round160_realistic_role41_base():
    # The serial captured runner supplies the frozen source pin and fresh output
    # root in this module. Discovery alone never executes a materializer.
    import sys
    source_root = '/home/fry/artifacts/vrf40-role41-20261025-round160'
    if source_root not in sys.path:
        sys.path.insert(0, source_root)
    import test_gen26_materialize
    return test_gen26_materialize.RealisticRole41Tests


class Round160FullRole41MaterializeTests(_round160_realistic_role41_base()):
    """Full real-input regression registered in the normal module.

    Execute with the lead-owned frozen runner, which supplies exact source and
    output ownership. The five inherited methods are the accepted real tests.
    """


class Round161QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=90.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 91.0, 'baseline_used_percent': 9.0, 'consume_percent': 20.0, 'floor_remaining_percent': 71.0, 'observed_utc': '2026-10-25T17:03:30.824000+00:00', 'resets_at': 1793532099, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '6d0d225cf991d81489bc3ede3d936beda4825db83c40dccd195434a9aff46d7a')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=20.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=71.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(71.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((71.0, 0), (70.0, 0), (90.0, -11), (90.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_ten_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_ten_points':
                value['consume_percent'] = 10.0
            if case == 'rollback':
                value.update(remaining_percent=92.0, used_percent=8.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'91.0', b'90.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)
        self.assertEqual(unittest.defaultTestLoader.loadTestsFromTestCase(Round160FullRole41MaterializeTests).countTestCases(), 5)


def _round161_realistic_role57_base():
    import sys
    source_root = '/home/fry/artifacts/vrf40-gen26-20261025-round161'
    if source_root not in sys.path:
        sys.path.insert(0, source_root)
    import test_role57_materialize
    return test_role57_materialize.RealisticRole57Tests


class Round161FullRole57MaterializeTests(_round161_realistic_role57_base()):
    """Complete actual role57 path; captured runner supplies frozen pins/output."""


def _round162_full_role61_archive_base():
    import sys
    source_root = '/home/fry/artifacts/vrf40-role61-20261025-round162'
    if source_root not in sys.path:
        sys.path.insert(0, source_root)
    import test_role61_materialize
    return test_role61_materialize.FullRole61ArchiveTests


class Round162FullRole61ArchiveTests(_round162_full_role61_archive_base()):
    """Exact actual-prefix archive/read-schedule regressions; captured runner."""


class Round162QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=86.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 87.0, 'baseline_used_percent': 13.0, 'consume_percent': 20.0, 'floor_remaining_percent': 67.0, 'observed_utc': '2026-10-25T18:40:11.543000+00:00', 'resets_at': 1793532099, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '4913a624c31661e1e4730a3e5760393be028b3926d9478a77ebe6f3626eddd55')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=20.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=67.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(67.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((67.0, 0), (66.0, 0), (86.0, -11), (86.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_ten_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_ten_points':
                value['consume_percent'] = 10.0
            if case == 'rollback':
                value.update(remaining_percent=88.0, used_percent=12.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'87.0', b'86.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)
        self.assertEqual(unittest.defaultTestLoader.loadTestsFromTestCase(Round160FullRole41MaterializeTests).countTestCases(), 5)


from test_role60_materialize import FullRole60ReviewTests as _Round162Role60Base
class Round162FullRole60ReviewTests(_Round162Role60Base):
    pass


class Round163QuotaBudgetTests(unittest.TestCase):
    """Synthetic telemetry checks the exact new grant; no execution evidence."""

    def fixture(self, remaining=75.0, drift=0):
        import pa1_fixed40_constructors as c
        budget = {'baseline_remaining_percent': 76.0, 'baseline_used_percent': 24.0, 'consume_percent': 20.0, 'floor_remaining_percent': 56.0, 'observed_utc': '2026-10-26T00:10:10.704000+00:00', 'resets_at': 1793532099, 'schema_version': 1, 'session_id': '01a14b66-bb49-7e22-8a8c-1cb04c03816e'}
        body = (json.dumps(budget, sort_keys=True) + '\n').encode()
        identity = hashlib.sha256(body).hexdigest()
        self.assertEqual(identity, '646b61ab9c056d8c418ba33826b903a8b844817c39db7c79ebe0040de8cdf575')
        consumed = budget['baseline_remaining_percent'] - remaining
        value = dict(age_seconds=0, baseline_observed_utc=budget['observed_utc'], baseline_remaining_percent=budget['baseline_remaining_percent'], baseline_used_percent=budget['baseline_used_percent'], baseline_resets_at=budget['resets_at'], baseline_session_id=budget['session_id'], session_id=budget['session_id'], budget_file=c.QUOTA_BUDGET_IDENTITIES[identity], budget_initialized=False, consume_percent=20.0, consumed_percent=consumed, consumed_percentage_points=consumed, effort='medium', floor_remaining_percent=56.0, log='/home/fry/artifacts/NONHISTORICAL-telemetry', model='gpt-6.1-sol', observed_utc=budget['observed_utc'], reason='NONHISTORICAL fixture', remaining_percent=remaining, used_percent=100.0 - remaining, reset_drift_seconds=drift, reset_tolerance_seconds=10, resets_at=budget['resets_at'] + drift, status='allow_start')
        return (c, body, value)

    def test_new_identity_floor_and_anchored_jitter(self):
        for drift in (-10, 0, 10):
            c, budget, value = self.fixture(56.1, drift)
            raw = json.dumps(value).encode()
            self.assertEqual(c.quota_snapshot(raw, maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc']), raw)
        for remaining, drift in ((56.0, 0), (55.0, 0), (75.0, -11), (75.0, 11)):
            c, budget, value = self.fixture(remaining, drift)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_no_pooling_rollback_or_rebinding(self):
        for case in ('pooled_baseline', 'pooled_floor', 'old_ten_points', 'rollback', 'forged_drift', 'unknown_budget'):
            c, budget, value = self.fixture()
            if case == 'pooled_baseline':
                value['baseline_remaining_percent'] = 99.0
            if case == 'pooled_floor':
                value['floor_remaining_percent'] = 69.0
            if case == 'old_ten_points':
                value['consume_percent'] = 10.0
            if case == 'rollback':
                value.update(remaining_percent=77.0, used_percent=23.0, consumed_percent=-1.0, consumed_percentage_points=-1.0)
            if case == 'forged_drift':
                value['reset_drift_seconds'] = 1
            if case == 'unknown_budget':
                budget = budget.replace(b'76.0', b'75.0', 1)
            with self.assertRaises(ValueError):
                c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])

    def test_expiry_and_older_identity_are_preserved(self):
        from datetime import datetime, timezone
        c, budget, value = self.fixture()
        value['observed_utc'] = datetime.fromtimestamp(value['baseline_resets_at'], timezone.utc).isoformat()
        with self.assertRaises(ValueError):
            c.quota_snapshot(json.dumps(value).encode(), maxima=c.ROLE_MAXIMA, budget_raw=budget, live_bytes=0, now_utc=value['observed_utc'])
        old = '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4'
        self.assertEqual(c.QUOTA_BUDGET_IDENTITIES[old], '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json')
        self.assertEqual(c.QUOTA_BUDGET_WIDTHS[old], 277)
        self.assertEqual(c.QUOTA_BUDGET_RESET_TOLERANCES[old], 10)
        self.assertEqual(unittest.defaultTestLoader.loadTestsFromTestCase(Round160FullRole41MaterializeTests).countTestCases(), 5)
