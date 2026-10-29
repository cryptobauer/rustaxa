"""PA1 offline admission regressions. No node, guard CLI or database access."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import pa1_evidence as pa1


class PA1Tests(unittest.TestCase):
    def test_serialized_repeated_rows_compare_exact_bytes_without_reparsing_parent(self):
        pin=self.body(b'complete original body')
        rows=tuple((i,(pin['path'],pin['bytes'],pin['sha256']),'evidence','bytes') for i in range(12))
        ids=('many-first','many-second')
        wires=(pa1.canonical(rows),pa1.canonical(rows))
        context=dict(self.context,manifest=pa1.frozen_manifest_digest(ids,wires))
        nonbinary=2*(len(wires[0])+12*(pin['bytes']+1))
        call=pa1.Invocation(self.counters,context,0,ids,wires,nonbinary,0,
                            serialized=True,release_completed_wires=True)
        self.assertEqual(list(call.metadata_operations),[1,1])
        for identity in ids:
            with call.unit(identity) as unit:
                for occurrence,physical,kind,mode in unit.declarations:
                    unit.consume(occurrence,dict(path=physical[0],bytes=physical[1],sha256=physical[2]),kind=kind)
            self.assertEqual(call.declarations[call.next-1],b'')
        self.assertEqual(list(call.metadata_operations),[2,2])
        self.assertEqual(json.loads(call.complete())['unit_verdicts'],list(ids))

    def test_serialized_offset_identity_comparison_rejects_changed_mode(self):
        pin=self.body(b'complete original body')
        first=((5,(pin['path'],pin['bytes'],pin['sha256']),'evidence','bytes'),)
        other=((5,(pin['path'],pin['bytes'],pin['sha256']),'source','bytes'),)
        ids=('first','changed');wires=(pa1.canonical(first),pa1.canonical(other))
        context=dict(self.context,manifest=pa1.frozen_manifest_digest(ids,wires))
        with self.assertRaisesRegex(ValueError,'conflicting occurrence'):
            pa1.Invocation(self.counters,context,0,ids,wires,10000,0,serialized=True)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.run = '1'*64
        self.history = '7'*64
        self.counters = pa1.Counters.initialize(self.root/'counters', self.run, consumed_events=(0, 0, 0), event_history=self.history)
        self.context = dict(run=self.run, manifest='2'*64, source='3'*64,
                            verifier='4'*64, executables=('5'*64, '6'*64),
                            phase='offline', route='archive', policy=pa1.POLICY_SHA,
                            ledger=pa1.LEDGER_SHA, setup_policy=pa1.SETUP_POLICY_SHA,
                            setup_ledger=pa1.SETUP_LEDGER_SHA,
                            setup_publication=pa1.SETUP_PUBLICATION_SHA, event_history=self.history)

    def test_s3_exact_lifecycle_bound_and_durable_limit_identity(self):
        self.assertEqual(pa1.NONBINARY_LIFETIME, 35964936624)
        self.counters.admit(nonbinary=pa1.NONBINARY_LIFETIME)
        resumed = pa1.Counters(self.root/'counters', self.run, self.history)
        with self.assertRaisesRegex(ValueError, 'lifetime nonbinary'):
            resumed.admit(nonbinary=1)
        with patch.object(pa1, 'SETUP_ENVELOPE', pa1.SETUP_ENVELOPE+1):
            with self.assertRaisesRegex(ValueError, 'counter identity'):
                pa1.Counters(self.root/'counters', self.run, self.history)

    def test_s2_context_rejects_stale_setup_publication_before_admission(self):
        self.context['setup_publication'] = '0'*64
        with self.assertRaisesRegex(ValueError, 'invocation context'):
            self.invocation()
        with self.counters._locked():
            self.assertEqual(self.counters._read()['invocations'], 0)

    def invocation(self, units=('a',), nonbinary=4096, binary=0, clock=lambda: 0,
                   repeats=None, kind='evidence', mode='bytes'):
        pin = self.pin if hasattr(self, 'pin') else self.body(b'x')
        row = (0, (pin['path'], pin['bytes'], pin['sha256']), kind, mode)
        declarations = tuple((row,)*(repeats[i] if repeats else 1) for i in range(len(units)))
        with self.counters._locked():
            state = self.counters._read()
        route = next((name for name, cap in pa1.ROUTES.items() if state['routes'][0][name] < cap), 'seal')
        context = dict(self.context, manifest=pa1.digest((units, declarations)), route=route)
        return pa1.Invocation(self.counters, context, 0, units, declarations, nonbinary, binary, clock)

    def body(self, raw=b'{"ok":true}', name='body'):
        path = self.root/name
        path.write_bytes(raw)
        self.pin = dict(path=str(path), bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest())
        return self.pin

    def test_complete_only_after_ordered_full_consumption(self):
        pin = self.body()
        call = self.invocation()
        with call.unit('a') as unit:
            raw = unit.consume(0, pin, retain=True)
            self.assertEqual(json.loads(raw), {'ok': True})
        receipt = json.loads(call.complete())
        self.assertFalse(receipt['authority'])
        self.assertEqual(receipt['unit_verdicts'], ['a'])
        with self.assertRaises(ValueError):
            call.complete()

    def test_bounded_projection_authenticates_full_containing_stream(self):
        value = b'{"complete":true,"roles":[1,2,3]}'
        prefix = b'x'*(pa1.CHUNK-3)
        pin = self.body(prefix+value+b'y'*(3*pa1.CHUNK))
        mode = 'slice:'+str(len(prefix))+':'+str(len(value))
        metadata = len(pa1.canonical(((0, (pin['path'], pin['bytes'], pin['sha256']), 'evidence', mode),)))
        call = self.invocation(nonbinary=pin['bytes']+1+metadata, mode=mode)
        with call.unit('a') as unit:
            result = unit.decode_slice(0, pin, len(prefix), len(value), hashlib.sha256(value).hexdigest())
            self.assertEqual(result, {'complete': True, 'roles': [1, 2, 3]})
            self.assertEqual(unit.nonbinary, pin['bytes']+1+metadata)
            self.assertEqual(call.operations[0], 2)
        self.assertEqual(json.loads(call.complete())['unit_verdicts'], ['a'])

    def test_projection_rejects_outside_mutation_and_nonexact_eof(self):
        value = b'{"whole_body_required":true}'
        original = b'x'*100+value+b'y'*200000
        for changed in (b'z'+original[1:], original+b'z'):
            pin = self.body(original)
            call = self.invocation(nonbinary=pin['bytes']+4096, mode='slice:100:'+str(len(value)))
            Path(pin['path']).write_bytes(changed)
            with self.assertRaises(ValueError):
                with call.unit('a') as unit:
                    unit.decode_slice(0, pin, 100, len(value), hashlib.sha256(value).hexdigest())
            with self.assertRaises(ValueError):
                call.complete()

    def test_projection_rejects_wrong_slice_hash_and_excess_buffer(self):
        value = b'{"exact":true}'
        pin = self.body(b'x'*10+value+b'y'*150000)
        call = self.invocation(nonbinary=pin['bytes']+4096, mode='slice:10:'+str(len(value)))
        with self.assertRaisesRegex(ValueError, 'projection hash'):
            with call.unit('a') as unit:
                unit.decode_slice(0, pin, 10, len(value), '0'*64)
        call = self.invocation(nonbinary=pin['bytes']+4096, mode='slice:0:131073')
        with self.assertRaises(ValueError):
            with call.unit('a') as unit:
                unit.decode_slice(0, pin, 0, 131073, '0'*64)

    def test_incomplete_or_wrong_order_never_completes(self):
        call = self.invocation(('a', 'b'))
        with call.unit('a') as unit:
            unit.consume(0, self.pin)
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            call.complete()
        call = self.invocation()
        with self.assertRaisesRegex(ValueError, 'unit order'):
            with call.unit('b'):
                pass
        with self.assertRaises(ValueError):
            call.complete()

    def test_same_size_mutation_and_restore_do_not_validate_consumed_bytes(self):
        pin = self.body(b'good')
        call = self.invocation()
        real_read = pa1.os.read
        changed = False
        def read(fd, size):
            nonlocal changed
            raw = real_read(fd, size)
            if not changed and raw:
                changed = True
                Path(pin['path']).write_bytes(b'good')
                return b'evil'
            return raw
        with patch.object(pa1.os, 'read', side_effect=read):
            with self.assertRaisesRegex(ValueError, 'consumed stream hash'):
                with call.unit('a') as unit:
                    unit.consume(0, pin, retain=True)
        self.assertEqual(Path(pin['path']).read_bytes(), b'good')
        with self.assertRaises(ValueError):
            call.complete()

    def test_returned_authenticated_buffer_survives_later_path_substitution(self):
        pin = self.body(b'good')
        call = self.invocation(repeats=(2,))
        with self.assertRaisesRegex(ValueError, 'failed/closed'):
            with call.unit('a') as unit:
                raw = unit.consume(0, pin, retain=True)
                Path(pin['path']).write_bytes(b'evil')
                self.assertEqual(raw, b'good')
                with self.assertRaisesRegex(ValueError, 'consumed stream hash'):
                    unit.consume(0, pin, retain=True)
        with self.assertRaises(ValueError):
            call.complete()

    def test_invocation_wide_occurrence_limit_does_not_reset_at_unit_boundary(self):
        pin = self.body(b'x')
        call = self.invocation(('a', 'b'), repeats=(8, 1))
        with call.unit('a') as unit:
            for _ in range(8):
                unit.consume(0, pin)
        with self.assertRaisesRegex(ValueError, 'ninth'):
            with call.unit('b') as unit:
                unit.consume(0, pin)
        with self.assertRaises(ValueError):
            call.complete()

    def test_failure_keeps_pre_admitted_debits_after_restart(self):
        pin = self.body(b'good')
        Path(pin['path']).unlink()
        call = self.invocation(nonbinary=1234)
        with self.assertRaises(FileNotFoundError):
            with call.unit('a') as unit:
                unit.consume(0, pin)
        resumed = pa1.Counters(self.root/'counters', self.run, self.history)
        with resumed._locked():
            state = resumed._read()
        self.assertEqual((state['invocations'], state['nonbinary'], state['seconds']), (1, 1234, 300))
        with self.assertRaises(FileExistsError):
            pa1.Counters.initialize(self.root/'counters', self.run, consumed_events=(0, 0, 0), event_history=self.history)

    def test_missing_corrupt_and_unknown_counters_fail_closed(self):
        with self.assertRaises(ValueError):
            pa1.Counters(self.root/'counters', 'f'*64, self.history)
        path = self.root/'counters/state.json'
        path.write_bytes(b'{}')
        with self.assertRaises(ValueError):
            pa1.Counters(self.root/'counters', self.run, self.history)
        path.unlink()
        with self.assertRaises(FileNotFoundError):
            pa1.Counters(self.root/'counters', self.run, self.history)

    def test_unit_source_cap_rejects_before_read(self):
        pin = self.body(b'x')
        pin['bytes'] = pa1.SOURCE+1
        call = self.invocation(nonbinary=pin['bytes'], kind='source')
        real_open = pa1.os.open
        def checked_open(path, *args, **kwargs):
            if str(path) == pin['path']:
                raise AssertionError('body must not open')
            return real_open(path, *args, **kwargs)
        with patch.object(pa1.os, 'open', side_effect=checked_open):
            with self.assertRaisesRegex(ValueError, 'unit/invocation bytes'):
                with call.unit('a') as unit:
                    unit.consume(0, pin, 'source')

    def test_deadline_is_shared_by_all_units(self):
        now = [0]
        call = self.invocation(('a', 'b'), clock=lambda: now[0])
        with call.unit('a') as unit:
            unit.consume(0, self.pin)
        now[0] = 300
        with self.assertRaisesRegex(ValueError, 'deadline'):
            with call.unit('b'):
                pass

    def test_slot_invocation_and_failure_caps_survive_restart(self):
        for _ in range(16):
            call = self.invocation()
            with call.unit('a') as unit:
                unit.consume(0, self.pin)
            call.complete()
        with self.assertRaisesRegex(ValueError, 'route exhaustion|slot exhaustion'):
            self.invocation()
        for _ in range(8):
            self.counters.admit(failure_slot=0)
        with self.assertRaisesRegex(ValueError, 'failure exhaustion'):
            self.counters.admit(failure_slot=0)

    def test_stage_reserves_every_future_maximum_and_all_live_overlap(self):
        maxima = (1048576,)*81
        actual = (None,)*81
        result = pa1.stage_reservation(maxima, actual, (0, 1, 2, 3), (4,))
        self.assertEqual(result['lifecycle_reserved_bytes'], 81*1048576)
        self.assertEqual(result['live_bytes'], pa1.LIVE)
        with self.assertRaisesRegex(ValueError, 'live evidence overflow'):
            pa1.stage_reservation(maxima, actual, (0, 1, 2, 3, 4), (5,))
        with self.assertRaisesRegex(ValueError, 'duplicate role'):
            pa1.stage_reservation(maxima, actual, (0,), (0,))

    def test_completion_rejects_changed_source_binding(self):
        call = self.invocation()
        with call.unit('a') as unit:
            unit.consume(0, self.pin)
        call.context['source'] = '9'*64
        with self.assertRaisesRegex(ValueError, 'completion binding'):
            call.complete()

    def test_oversized_buffer_rejects_before_open(self):
        pin = self.body()
        call = self.invocation()
        real_open = pa1.os.open
        def checked_open(path, *args, **kwargs):
            if str(path) == pin['path']:
                raise AssertionError('body must not open')
            return real_open(path, *args, **kwargs)
        with patch.object(pa1.os, 'open', side_effect=checked_open):
            with self.assertRaisesRegex(ValueError, 'allocation'):
                with call.unit('a') as unit:
                    unit.consume(0, pin, retain=True, live_bytes=pa1.SEMANTIC)

    def test_stale_unit_handle_cannot_bypass_next_units_caps(self):
        self.body(b'x')
        call = self.invocation(('a', 'b'))
        with call.unit('a') as first:
            first.consume(0, self.pin)
        with self.assertRaisesRegex(ValueError, 'occurrence identity'):
            with call.unit('b'):
                first.consume(0, self.pin)
        with self.assertRaises(ValueError):
            call.complete()

    def test_empty_unit_cannot_discharge_frozen_read(self):
        call = self.invocation()
        with self.assertRaisesRegex(ValueError, 'incomplete unit closure'):
            with call.unit('a'):
                pass
        with self.assertRaises(ValueError):
            call.complete()

    def test_frozen_role_pin_and_kind_cannot_be_substituted(self):
        self.body(b'x')
        for replacement in [(1, self.pin, 'evidence'), (0, self.pin, 'source'),
                            (0, dict(self.pin, sha256='0'*64), 'evidence')]:
            call = self.invocation()
            with self.assertRaisesRegex(ValueError, 'frozen read order'):
                with call.unit('a') as unit:
                    unit.consume(*replacement)

    def test_json_parse_is_mandatory_and_charged_separately(self):
        pin = self.body(b'{"ok":true}')
        call = self.invocation(mode='json')
        with self.assertRaisesRegex(ValueError, 'frozen read order'):
            with call.unit('a') as unit:
                unit.consume(0, pin, retain=True)
        call = self.invocation(mode='json')
        with call.unit('a') as unit:
            self.assertEqual(unit.decode(0, pin), {'ok': True})
        self.assertEqual(call.operations[0], 2)
        call.complete()

    def test_streamed_body_still_charges_live_graph(self):
        pin = self.body(b'x')
        call = self.invocation()
        real_open = pa1.os.open
        def checked_open(path, *args, **kwargs):
            if str(path) == pin['path']:
                raise AssertionError('body must not open')
            return real_open(path, *args, **kwargs)
        with patch.object(pa1.os, 'open', side_effect=checked_open):
            with self.assertRaisesRegex(ValueError, 'live allocation'):
                with call.unit('a') as unit:
                    unit.consume(0, pin, live_bytes=pa1.SEMANTIC)

    def test_history_closure_checks_distinct_frame_and_whole_container(self):
        raw = b'one complete member'
        member_path = '/home/fry/artifacts/pa1-test-logical-member'
        role = 'scope:3'
        checksum = hashlib.sha256(raw).hexdigest()
        frame = (b'N42HIS2\x00'+(0).to_bytes(2, 'little')+(1).to_bytes(2, 'little')
                 +(0).to_bytes(4, 'little')+len(role).to_bytes(2, 'little')
                 +len(member_path).to_bytes(2, 'little')+len(raw).to_bytes(8, 'little')
                 +bytes.fromhex(checksum)+role.encode()+member_path.encode()+raw)
        pin = self.body(frame)
        rows = ((1, (pin['path'], pin['bytes'], pin['sha256']), 'evidence', 'history:0'),
                (0, (member_path, len(raw), checksum), 'evidence', 'member:'+role))
        declarations = (rows,)
        ids = ('history:0',)
        context = dict(self.context, manifest=pa1.digest((ids, declarations)))
        call = pa1.Invocation(self.counters, context, 0, ids, declarations, len(frame)+1+len(pa1.canonical(rows)), 0, lambda: 0)
        with call.unit('history:0') as unit:
            unit.history(1, pin, 0, 0, 1)
        call.complete()
        self.assertEqual(list(call.operations[:2]), [1, 2])
        self.assertEqual(call.nonbinary_remaining, 0)
        context = dict(context, route='pre-gate')
        call = pa1.Invocation(self.counters, context, 0, ids, declarations,
                              len(frame)+1+len(pa1.canonical(rows)), 0, lambda: 0)
        call.operations[1] = 7
        original_open = pa1.os.open
        def open_without_body(path, *args, **kwargs):
            if str(path) == pin['path']:
                raise AssertionError('ninth operation must stop before file read')
            return original_open(path, *args, **kwargs)
        with patch.object(pa1.os, 'open', side_effect=open_without_body):
            with self.assertRaisesRegex(ValueError, 'ninth history operation'):
                with call.unit('history:0') as unit:
                    unit.history(1, pin, 0, 0, 1)

    def test_failed_json_semantics_never_complete(self):
        pin = self.body(b'{"duplicate":1,"duplicate":2}')
        call = self.invocation(mode='json')
        with self.assertRaisesRegex(ValueError, 'duplicate JSON key'):
            with call.unit('a') as unit:
                unit.decode(0, pin)
        with self.assertRaises(ValueError):
            call.complete()

    def test_interrupted_pending_invocation_cannot_reset_on_restart(self):
        self.invocation()
        resumed = pa1.Counters(self.root/'counters', self.run, self.history)
        with self.assertRaisesRegex(ValueError, 'unresolved prior invocation'):
            resumed.admit(invocation=1, units=1, nonbinary=1, seconds=300,
                          slot=0, route='pre-gate')

    def test_route_budget_cannot_be_spent_as_extra_archive_attempts(self):
        call = self.invocation()
        with call.unit('a') as unit:
            unit.consume(0, self.pin)
        call.complete()
        with self.assertRaisesRegex(ValueError, 'route exhaustion'):
            self.counters.admit(invocation=1, units=1, nonbinary=1,
                                seconds=300, slot=0, route='archive')

    def test_manifest_binding_streams_without_complete_canonical_buffer(self):
        value = (('a', 'b'), (((0, ('/home/fry/artifacts/body', 1, '0'*64),
                                'evidence', 'bytes'),),))
        expected = hashlib.sha256(pa1.canonical(value)).hexdigest()
        with patch.object(pa1, 'canonical', side_effect=AssertionError('whole graph encoder')):
            self.assertEqual(pa1.digest(value), expected)

    def test_manifest_transfer_overflow_stops_before_durable_admission(self):
        pin = self.body(b'x')
        row = (0, (pin['path'], pin['bytes'], pin['sha256']), 'evidence', 'bytes')
        # The decoded manifest fits by itself; encoder and retained output
        # overlap must also fit. Long distinct unit IDs exercise that lifetime.
        ids = tuple(str(i).zfill(120) for i in range(800))
        declarations = tuple((row,) for _ in ids)
        context = dict(self.context, manifest=pa1.digest((ids, declarations)))
        with self.assertRaisesRegex(ValueError, 'manifest transfer allocation'):
            pa1.Invocation(self.counters, context, 0, ids, declarations, 1000000, 0)
        with self.counters._locked():
            self.assertEqual(self.counters._read()['invocations'], 0)

    def test_2049_units_reject_before_durable_admission(self):
        pin = self.body(b'x')
        row = (0, (pin['path'], pin['bytes'], pin['sha256']), 'evidence', 'bytes')
        ids = tuple(str(i) for i in range(2049))
        declarations = tuple((row,) for _ in ids)
        with self.assertRaisesRegex(ValueError, 'ordered unit identities'):
            pa1.Invocation(self.counters, self.context, 0, ids, declarations, 10000, 0)
        with self.counters._locked():
            self.assertEqual(self.counters._read()['invocations'], 0)

    def test_consumed_setup_events_cannot_be_reset_by_activation(self):
        counters = pa1.Counters.initialize(self.root/'activation', self.run,
                                          consumed_events=(8, 0, 0), event_history=self.history)
        with self.assertRaisesRegex(ValueError, 'no reserved failure outcome'):
            counters.admit(invocation=1, units=1, nonbinary=1,
                           seconds=300, slot=0, route='archive')
        with self.assertRaisesRegex(ValueError, 'counter identity'):
            pa1.Counters(self.root/'activation', self.run, '8'*64)

    def test_changed_per_unit_limit_cannot_resume_durable_state(self):
        with patch.object(pa1, 'SOURCE', pa1.SOURCE+1):
            with self.assertRaisesRegex(ValueError, 'counter identity'):
                pa1.Counters(self.root/'counters', self.run, self.history)

    def test_s3_preserves_four_consumed_events_across_restart_without_reset(self):
        counters = pa1.Counters.initialize(self.root/'s3-activation', self.run,
                                          consumed_events=(4, 0, 0), event_history=self.history)
        counters = pa1.Counters(self.root/'s3-activation', self.run, self.history)
        with counters._locked():
            self.assertEqual(counters._read()['failures'], [4, 0, 0])
        for expected in range(5, 9):
            counters.admit(failure_slot=0)
            counters = pa1.Counters(self.root/'s3-activation', self.run, self.history)
            with counters._locked():
                self.assertEqual(counters._read()['failures'][0], expected)
        before = (self.root/'s3-activation/state.json').read_bytes()
        with self.assertRaisesRegex(ValueError, 'failure exhaustion'):
            counters.admit(failure_slot=0)
        self.assertEqual((self.root/'s3-activation/state.json').read_bytes(), before)
        with self.assertRaises(FileExistsError):
            pa1.Counters.initialize(self.root/'s3-activation', self.run,
                                   consumed_events=(0, 0, 0), event_history=self.history)

    def test_s3_leaves_later_slot_four_event_limits_and_requires_known_history(self):
        for slot in (1, 2):
            for _ in range(4):
                self.counters.admit(failure_slot=slot)
            with self.assertRaisesRegex(ValueError, 'failure exhaustion'):
                self.counters.admit(failure_slot=slot)
        with self.assertRaises(ValueError):
            pa1.Counters.initialize(self.root/'unknown', self.run,
                                   consumed_events=None, event_history=self.history)
        self.assertFalse((self.root/'unknown').exists())

    def test_s3_failed_combined_admission_does_not_partially_debit(self):
        self.counters.admit(nonbinary=pa1.NONBINARY_LIFETIME)
        before = (self.root/'counters/state.json').read_bytes()
        with self.assertRaisesRegex(ValueError, 'lifetime nonbinary'):
            self.counters.admit(invocation=1, units=1, nonbinary=1,
                                seconds=300, slot=0, route='archive')
        self.assertEqual((self.root/'counters/state.json').read_bytes(), before)

    def test_invocation_and_last_failure_cannot_be_debited_together(self):
        for _ in range(7):
            self.counters.admit(failure_slot=0)
        before = (self.root/'counters/state.json').read_bytes()
        with self.assertRaisesRegex(ValueError, 'cannot also consume a failure event'):
            self.counters.admit(invocation=1, units=1, nonbinary=1, seconds=300,
                                slot=0, route='archive', failure_slot=0)
        self.assertEqual((self.root/'counters/state.json').read_bytes(), before)

    def test_same_retained_stream_parse_debit_and_unit_release(self):
        pin = self.body(b'{"ok":true}')
        call = self.invocation()
        with call.unit('a') as unit:
            raw = unit.consume(0, pin, retain=True)
            self.assertEqual(unit.decode_consumed(0, raw), {'ok': True})
            self.assertEqual(call.operations[0], 2)
        self.assertIsNone(unit._retained)
        self.assertEqual(unit.declarations, ())
        call.complete()

    def test_copied_or_unrelated_buffer_cannot_inherit_consumed_authentication(self):
        pin = self.body(b'{"ok":true}')
        call = self.invocation()
        with self.assertRaisesRegex(ValueError, 'same consumed buffer identity'):
            with call.unit('a') as unit:
                raw = unit.consume(0, pin, retain=True)
                copied = bytes(bytearray(raw))
                unit.decode_consumed(0, copied)

    def test_exact_eof_attempt_is_reserved_before_body_read(self):
        pin = self.body(b'x')
        call = self.invocation(nonbinary=len(pa1.canonical(((0, (pin['path'], 1, pin['sha256']), 'evidence', 'bytes'),)))+1)
        original_open = pa1.os.open
        def reject_body(path, *args, **kwargs):
            if str(path) == pin['path']:
                raise AssertionError('EOF must be reserved before physical read')
            return original_open(path, *args, **kwargs)
        with patch.object(pa1.os, 'open', side_effect=reject_body):
            with self.assertRaisesRegex(ValueError, 'unit/invocation bytes'):
                with call.unit('a') as unit:
                    unit.consume(0, pin)

    def test_alias579_is_exact_and_preserves_logical_declaration(self):
        logical = dict(path='/workspaces/rustaxa-evm/doc/evm_research/n4_evidence_record_successor.md',
                       bytes=4446, sha256='00f7c7d32e283acd5fc535c94e634b377f7211897d765ef86e75717ef02cbe4e')
        physical = pa1.fixed579_physical_pin(579, logical)
        self.assertNotEqual(physical['path'], logical['path'])
        self.assertEqual(physical['sha256'], logical['sha256'])
        self.assertIs(pa1.fixed579_physical_pin(578, logical), logical)
        changed = dict(logical, sha256='0'*64)
        self.assertIs(pa1.fixed579_physical_pin(579, changed), changed)

    def test_control_float_parse_requires_exact_consumed_operational_role(self):
        pin = self.body(b'{"elapsed_seconds":0.25}')
        log = self.body(b'complete saved output', 'saved.log')
        ids = ('selected40-semantic:2',)
        def rows(pin):
            return (((684, (pin['path'], pin['bytes'], pin['sha256']), 'evidence', 'bytes'),
                     (685, (log['path'], log['bytes'], log['sha256']), 'evidence', 'bytes')),)
        declarations = rows(pin)
        context = dict(self.context, manifest=pa1.digest((ids, declarations)))
        call = pa1.Invocation(self.counters, context, 0, ids, declarations, 4096, 0)
        with call.unit(ids[0]) as unit:
            raw = unit.consume(684, pin, retain=True)
            self.assertEqual(unit.decode_control_consumed(684, raw, 'selected40-saved-result'),
                             {'elapsed_seconds': 0.25})
            unit.consume(685, log)
        call.complete()
        for raw in (b'{"elapsed_seconds":NaN}', b'{"elapsed_seconds":1e999}'):
            pin = self.body(raw)
            declarations = rows(pin)
            context = dict(self.context, route='command', manifest=pa1.digest((ids, declarations)))
            call = pa1.Invocation(self.counters, context, 0, ids, declarations, 4096, 0)
            with self.assertRaisesRegex(ValueError, 'nonfinite control scalar'):
                with call.unit(ids[0]) as unit:
                    body = unit.consume(684, pin, retain=True)
                    unit.decode_control_consumed(684, body, 'selected40-saved-result')

    def test_control_decoder_rejects_other_report_in_same_frozen_unit(self):
        gate = self.body(b'{"elapsed_seconds":0.25}')
        report = self.body(b'{"report_metric":0.5}', 'report.json')
        ids = ('selected40-semantic:2',)
        rows = (((684, (gate['path'], gate['bytes'], gate['sha256']), 'evidence', 'bytes'),
                 (685, (report['path'], report['bytes'], report['sha256']), 'evidence', 'bytes')),)
        context = dict(self.context, manifest=pa1.digest((ids, rows)))
        call = pa1.Invocation(self.counters, context, 0, ids, rows, 4096, 0)
        with self.assertRaisesRegex(ValueError, 'exact operational role'):
            with call.unit(ids[0]) as unit:
                unit.consume(684, gate)
                raw = unit.consume(685, report, retain=True)
                unit.decode_control_consumed(685, raw, 'selected40-saved-result')

    def test_cold_counter_scope_tracks_six_actual_operations_and_versions(self):
        counters = pa1.Counters.initialize(self.root/'traced', self.run,
            consumed_events=(4, 0, 0), event_history=self.history, trace_io=True)
        pin = self.body(b'x')
        ids = ('a',)
        rows = (((0, (pin['path'], 1, pin['sha256']), 'evidence', 'bytes'),),)
        context = dict(self.context, manifest=pa1.digest((ids, rows)))
        counters.begin_io_scope()
        call = pa1.Invocation(counters, context, 0, ids, rows, 4096, 0)
        self.assertEqual(call.consumed_events, (4, 0, 0))
        with call.unit('a') as unit:
            unit.consume(0, pin)

        call.complete()
        summary = counters.io_summary()
        self.assertEqual(summary['physical_operations'], 6)
        self.assertLessEqual(summary['attempted_bytes'], 24582)
        self.assertEqual(summary['reserved_bytes'], 32768)
        self.assertTrue(all(row['authenticated'] for row in summary['versions']))
        self.assertTrue(all(len(row['sha256']) == 64 for row in summary['versions']))

    def test_serialized_manifest_removes_complete_graph_transfer_overlap(self):
        pin = self.body(b'x')
        ids = tuple('closure:'+str(i) for i in range(900))
        rows = tuple(((i, (pin['path'], 1, pin['sha256']), 'evidence', 'bytes'),) for i in range(900))
        wires = tuple(pa1.canonical(row) for row in rows)
        expected = pa1.digest((ids, rows))
        self.assertEqual(pa1.frozen_manifest_digest(ids, wires), expected)
        context = dict(self.context, manifest=expected)
        with self.assertRaisesRegex(ValueError, 'manifest transfer allocation'):
            pa1.Invocation(self.counters, context, 0, ids, rows, 1048576, 0)
        del rows
        call = pa1.Invocation(self.counters, context, 0, ids, wires, 1048576, 0,
                              serialized=True, live_bytes=131072)
        self.assertIs(call.declarations, wires)
        self.assertLess(call.metadata_charge(), pa1.SEMANTIC)
        self.assertEqual(call.caller_charge, 131072)
        self.assertGreaterEqual(call.metadata_charge(), call.manifest_charge+131072)
        call.fail()

    def test_serialized_repeated_metadata_keeps_later_parse_within_eight(self):
        pin = self.body(b'x')
        ids = tuple('repeat:'+str(i) for i in range(9))
        row = (0, (pin['path'], 1, pin['sha256']), 'evidence', 'bytes')
        wires = tuple(pa1.canonical((row,)) for _ in ids)
        context = dict(self.context, manifest=pa1.frozen_manifest_digest(ids, wires))
        call=pa1.Invocation(self.counters, context, 0, ids, wires, 4096, 0, serialized=True)
        self.assertEqual(list(call.metadata_operations),[1]*9)
        for identity in ids[:8]:
            with call.unit(identity) as unit:
                unit.consume(0,pin,kind='evidence')
        self.assertEqual(list(call.metadata_operations),[2]*8+[1])
        with self.assertRaisesRegex(ValueError, 'ninth'):
            with call.unit(ids[8]) as unit:
                unit.consume(0,pin,kind='evidence')

    def test_serialized_manifest_compares_complete_repeated_identity(self):
        pin = self.body(b'x')
        ids = ('first', 'second')
        row = (0, (pin['path'], 1, pin['sha256']), 'evidence', 'bytes')
        changed = (0, (pin['path'], 1, '0'*64), 'evidence', 'bytes')
        wires = (pa1.canonical((row,)), pa1.canonical((changed,)))
        context = dict(self.context, manifest=pa1.frozen_manifest_digest(ids, wires))
        with self.assertRaisesRegex(ValueError, 'conflicting occurrence identity'):
            pa1.Invocation(self.counters, context, 0, ids, wires, 4096, 0, serialized=True)

    def test_serialized_caller_graph_rejects_before_decode_or_admission(self):
        pin = self.body(b'x')
        ids = ('first',)
        row = (0, (pin['path'], 1, pin['sha256']), 'evidence', 'bytes')
        wires = (pa1.canonical((row,)),)
        context = dict(self.context, manifest=pa1.frozen_manifest_digest(ids, wires))
        with patch.object(pa1.records, 'decode', side_effect=AssertionError('decode before admission bound')):
            with self.assertRaisesRegex(ValueError, 'serialized manifest allocation'):
                pa1.Invocation(self.counters, context, 0, ids, wires, 4096, 0,
                               serialized=True, live_bytes=pa1.SEMANTIC)
    def test_receipt_unicode_wire_overflow_precedes_complete_serialization(self):
        pin = self.body(b'x')
        ids = tuple('\u20ac'*40+str(i) for i in range(550))
        declarations = tuple(((i, (pin['path'], pin['bytes'], pin['sha256']),
                              'evidence', 'bytes'),) for i in range(550))
        context = dict(self.context, manifest=pa1.digest((ids, declarations)))
        budget = 1100+sum(len(pa1.canonical(rows)) for rows in declarations)
        call = pa1.Invocation(self.counters, context, 0, ids, declarations, budget, 0)
        for i, identity in enumerate(ids):
            with call.unit(identity) as unit:
                unit.consume(i, pin)
        original = pa1.canonical
        def encode(value):
            if isinstance(value, dict) and value.get('schema') == 'pa1-completion-v1':
                raise AssertionError('oversized receipt must not be allocated')
            return original(value)
        with patch.object(pa1, 'canonical', side_effect=encode):
            with self.assertRaisesRegex(ValueError, 'receipt overflow before allocation'):
                call.complete()


if __name__ == '__main__':
    unittest.main()
