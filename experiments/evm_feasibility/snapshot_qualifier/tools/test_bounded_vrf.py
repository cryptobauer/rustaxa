"""Offline command-bound regressions. No guard main or DB inspection runs."""
import copy
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import bounded_vrf as guard
import evidence_records as records


def historical_instruction_fixture(test):
    """Authenticate exact old instruction bodies for offline legacy fixtures.

    Live/current/forged pins keep the real guard path. Only seven independently
    reviewed historical logical pins use their full fixed archived bytes. This
    fixture port preserves the original command assertions and production code.
    """
    original = guard.pin_file
    mapping = guard.fourth_record_contract()['instruction_mapping']
    if len(mapping) != 7:
        raise ValueError('exact fixture archive roster')

    def authenticate(path, expected=None, cap=guard.FILE_CAP):
        for item in mapping:
            archive = item['historical_archive']
            logical = dict(archive, path=item['logical_path'])
            if str(path) == item['logical_path'] and expected == logical:
                actual = original(archive['path'], archive, cap)
                return dict(actual, path=item['logical_path'])
        return original(path, expected, cap)

    active = patch.object(guard, 'pin_file', side_effect=authenticate)
    active.start()
    test.addCleanup(active.stop)
    return active


class Fixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='vrf-tool-test-', dir='/home/fry/artifacts')
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()


class HistoricalInstructionFixture(Fixture):
    def test_exact_archived_pin_forgery_caps_and_lifecycle(self):
        original = guard.pin_file
        mapping = guard.fourth_record_contract()['instruction_mapping']
        active = historical_instruction_fixture(self)
        item = mapping[0]
        expected = dict(item['historical_archive'], path=item['logical_path'])
        self.assertEqual(guard.pin_file(item['logical_path'], expected), expected)
        self.assertNotEqual(guard.pin_file(item['logical_path']), expected)
        with self.assertRaises(ValueError):
            guard.pin_file(item['logical_path'], dict(expected, sha256='0'*64))
        with self.assertRaises(ValueError):
            guard.pin_file(item['logical_path'], expected, expected['bytes']-1)
        with self.assertRaises((ValueError, FileNotFoundError)):
            guard.pin_file('/unreviewed/path', expected)
        active.stop()
        self.assertIs(guard.pin_file, original)
        with self.assertRaises(ValueError):
            guard.pin_file(item['logical_path'], expected)


class EvidenceSuccessor(Fixture):
    """Exercise the actual frozen shared verifier without physical inputs."""

    def fixture(self):
        source = Path(records.__file__).resolve()
        test = source.with_name('test_evidence_records.py')
        folder = self.root/'fourth-source'
        folder.mkdir()
        body = self.root/'full-failure'
        body.write_bytes(b'complete immutable body')
        contract_file = self.root/'contract'
        contract_file.write_bytes(b'fixed trusted contract')
        contract_pin = guard.pin_file(contract_file)
        paths = [source, test]
        for i in range(6):
            path = self.root/f'gate-{i}'
            path.write_bytes(b'full output')
            paths.append(path)
        roles = ['record-builder-source', 'record-builder-tests', 'record-tool-test-result',
                 'record-tool-test-log', 'current-python-gate-result', 'current-python-gate-log', 'current-python-final-result', 'current-python-final-log']
        pending = [dict(role=role, **guard.pin_file(path)) for role, path in zip(roles, paths)]
        instructions = [dict(role=f'instruction-{i}', **guard.pin_file(body)) for i in range(18)]
        code_path = folder/'code-and-gates-freeze.json'
        review = folder/'code-acceptance-review.md'
        closure = folder/'fourth-command-closure-review.md'
        placements = [dict(destination=str(review), members=[dict(role=f'failure-{i}', **guard.pin_file(body)) for i in range(12)]),
                      dict(destination=str(code_path), members=[dict(role=f'code-failure-{i}', **guard.pin_file(body)) for i in range(2)]),
                      dict(destination=str(closure), members=[dict(role=f'closure-failure-{i}', **guard.pin_file(body)) for i in range(6)])]
        contract = dict(reference_placements=placements, instruction_reference_rows=instructions,
                        pending_validation_rows=dict(roles=roles, paths=[str(p) for p in paths], caps=[65536]*8))
        for placement in placements:
            expected = placement['members'] + (pending + instructions if placement['destination'] == str(code_path) else [])
            wrapper = records.decode(records.build(placement['destination'], contract_pin, '', expected))
            if placement['destination'] == str(code_path):
                wrapper = dict(record_tool_pins=pending, evidence_record=wrapper)
            Path(placement['destination']).write_text(json.dumps(wrapper))
        evidence = [guard.pin_file(Path(p['destination'])) for p in placements]
        return contract, contract_pin, evidence, body, code_path

    def run_verifier(self, contract, contract_pin, evidence):
        with patch.object(guard, 'RESIDUAL_RUN', self.root), \
             patch.object(guard, 'FOURTH_RECORD_CONTRACT', contract_pin), \
             patch.object(guard, 'fourth_record_contract', return_value=contract):
            return guard.verify_fourth_records(evidence)

    def test_shared_frozen_verifier_full_occurrences_and_charges(self):
        contract, pin, evidence, body, _ = self.fixture()
        result = self.run_verifier(contract, pin, evidence)
        self.assertEqual(result['reference_count'], 49)
        self.assertEqual(result['streamed_bytes'], result['referenced_bytes'])
        self.assertGreater(result['referenced_bytes'], result['source_bytes'] + 38*body.stat().st_size)

    def test_same_size_child_drift_rejects_before_any_capture(self):
        contract, pin, evidence, body, _ = self.fixture()
        body.write_bytes(b'X'*body.stat().st_size)
        with self.assertRaisesRegex(ValueError, 'body pin drift'):
            self.run_verifier(contract, pin, evidence)

    def test_pending_unknown_role_or_source_path_rejects(self):
        contract, pin, evidence, _, code_path = self.fixture()
        code = json.loads(code_path.read_bytes())
        code['record_tool_pins'][0]['path'] = str(self.root/'unreviewed.py')
        code_path.write_text(json.dumps(code))
        evidence = [guard.pin_file(Path(row['path'])) for row in evidence]
        with self.assertRaisesRegex(ValueError, 'fixed record tooling row'):
            self.run_verifier(contract, pin, evidence)


class Bounds(Fixture):
    def trusted_freeze(self):
        freeze = self.root/'command-freeze.json'
        value=dict(schema=1,profile=guard.PROFILE,run=str(self.root),source_commit='0'*40,
                   cwd=str(guard.REPO),environment=guard.ENV,capture_command=['/bin/false'],
                   pins=[dict(path=str(self.root/'spec.json'),bytes=0,sha256='0'*64)],
                   pin_count=1,pinned_file_bytes=0,source_count=68,complete_source_bytes=0,
                   embedded_source_bytes=0,copy_metadata=[],accepted=False)
        freeze.write_bytes(guard.encoded(value,guard.FREEZE_CAP))
        return freeze,guard.pin_file(freeze)['sha256'],value

    def run_child(self, size, exit_code=0, script=None):
        script = script or f'import os; os.write(1,b"x"*{size}); raise SystemExit({exit_code})'
        result = guard.run_bounded([sys.executable, '-I', '-c', script], guard.ENV, self.root/'out.log', 0.1 if script.startswith('import time') or 'time.sleep' in script else 3)
        self.assertEqual((self.root/'out.log').stat().st_size, min(size,65536))
        self.assertEqual(result['log_bytes'],min(size,65536))
        return result

    def test_empty(self):
        self.assertEqual(self.run_child(0)['exit'],0)

    def test_sink_exact_cap(self):
        self.assertFalse(self.run_child(65536)['output_overflow'])

    def test_sink_one_over(self):
        self.assertTrue(self.run_child(65537)['output_overflow'])

    def test_child_failure(self):
        self.assertEqual(self.run_child(1024,7)['exit'],7)

    def test_deadline_silent(self):
        self.assertTrue(self.run_child(0,script='import time; time.sleep(2)')['wall_timeout'])

    def test_deadline_after_pipe_eof(self):
        self.assertTrue(self.run_child(0,script='import os,time; os.close(1); os.close(2); time.sleep(2)')['wall_timeout'])

    def test_metadata_boundary_and_duplicate_keys(self):
        p=self.root/'data.json'
        p.write_bytes(b'0'+b' '*65535)
        self.assertEqual(guard.read_json(p),0)
        p.write_bytes(b'0'+b' '*65536)
        with self.assertRaises(ValueError): guard.read_json(p)
        p.write_text('{"a":1,"a":2}')
        with self.assertRaises(ValueError): guard.read_json(p)

    def test_command_freeze_encoded_boundary(self):
        raw=guard.encoded({'value':''},131072)
        n=131072-len(raw)
        self.assertEqual(len(guard.encoded({'value':'x'*n},131072)),131072)
        with self.assertRaises(ValueError): guard.encoded({'value':'x'*(n+1)},131072)

    def test_hash_drift_growth_short_and_symlink(self):
        p=self.root/'pin'; p.write_bytes(b'abc'); pin=guard.pin_file(p)
        p.write_bytes(b'abd')
        with self.assertRaises(ValueError): guard.pin_file(p,pin)
        p.write_bytes(b'abcd')
        with self.assertRaises(ValueError): guard.pin_file(p,pin)
        p.write_bytes(b'ab')
        with self.assertRaises(ValueError): guard.pin_file(p,pin)
        alias=self.root/'alias'; alias.symlink_to(p)
        with self.assertRaises(ValueError): guard.pin_file(alias)

    def test_file_cap_and_collision_preserve_bytes(self):
        p=self.root/'pin'; p.write_bytes(b'abcd')
        with self.assertRaises(ValueError): guard.pin_file(p,cap=3)
        with self.assertRaises(FileExistsError): guard.exclusive(p,b'replace')
        self.assertEqual(p.read_bytes(),b'abcd')

    def test_database_paths_rejected_before_inspection(self):
        with patch.object(Path,'stat',side_effect=AssertionError('unexpected path stat')):
            for p in [guard.COPY/'anything',guard.REPO/'data/anything']:
                with self.assertRaises(ValueError): guard.pin_file(p)

    def test_database_aliases_reject_before_stat_or_lstat(self):
        aliases = [guard.REPO/'local/evm-state-db/alias/../snapshot-litenode-copy/file',
                   guard.REPO/'alias/../data/file']
        with patch.object(Path,'stat',side_effect=AssertionError('stat before lexical rejection')), \
             patch.object(Path,'lstat',side_effect=AssertionError('lstat before lexical rejection')), \
             patch.object(Path,'open',side_effect=AssertionError('open before lexical rejection')):
            for path in aliases:
                with self.assertRaises(ValueError): guard.pin_file(path)

    def test_trusted_attempt_consumed_on_offline_failure(self):
        freeze,sha,_=self.trusted_freeze()
        with patch.object(guard,'verify_files',side_effect=ValueError('source drift')) as verify:
            with self.assertRaisesRegex(ValueError,'source drift'): guard.capture(freeze,sha)
            self.assertTrue((self.root/'vrf-seventh-voter-window-attempt.json').is_file())
            with self.assertRaises(FileExistsError): guard.capture(freeze,sha)
            self.assertEqual(verify.call_count,1)

    def test_untrusted_freeze_rejects_before_marker(self):
        freeze,sha,value=self.trusted_freeze()
        with self.assertRaises(ValueError): guard.capture(freeze,'0'*64)
        self.assertFalse((self.root/'vrf-seventh-voter-window-attempt.json').exists())
        for bad in [dict(value,extra=True),dict(value,schema=2),dict(value,run=str(self.root/'other'))]:
            freeze.write_bytes(guard.encoded(bad,guard.FREEZE_CAP)); digest=guard.pin_file(freeze)['sha256']
            with self.assertRaises(ValueError): guard.capture(freeze,digest)
            self.assertFalse((self.root/'vrf-seventh-voter-window-attempt.json').exists())

    def test_output_collision_consumes_attempt_without_metadata(self):
        freeze,sha,_=self.trusted_freeze()
        (self.root/'vrf-seventh-voter-window.rcv2').write_bytes(b'keep')
        with patch.object(guard,'verify_files'),patch.object(guard,'build',return_value=freeze.read_bytes()), \
             patch.object(guard.subprocess,'check_output',return_value=b'0'*40), \
             patch.object(guard,'copy_metadata',side_effect=AssertionError('metadata after collision')):
            with self.assertRaisesRegex(ValueError,'output collision'): guard.capture(freeze,sha)
        self.assertTrue((self.root/'vrf-seventh-voter-window-attempt.json').is_file())
        self.assertEqual((self.root/'vrf-seventh-voter-window.rcv2').read_bytes(),b'keep')

    def test_fixed_metadata_records_two_passes_without_opens_or_discovery(self):
        paths=[guard.COPY,guard.COPY/'db/db',guard.COPY/'db/state_db']
        rows=[dict(path=str(p),device=1,inode=i,mode=2,mtime_ns=3) for i,p in enumerate(paths)]
        def fake_stat(p):
            self.assertIn(p,paths)
            return SimpleNamespace(st_dev=1,st_ino=paths.index(p),st_mode=2,st_mtime_ns=3)
        with patch.object(Path,'is_symlink',return_value=False),patch.object(Path,'resolve',lambda p:p), \
             patch.object(Path,'stat',autospec=True,side_effect=fake_stat) as stat, \
             patch.object(Path,'open',side_effect=AssertionError('metadata opened file')), \
             patch.object(Path,'iterdir',side_effect=AssertionError('metadata enumerated directory')):
            self.assertEqual(guard.copy_metadata(rows),rows)
            self.assertEqual(guard.copy_metadata(rows),rows)
            self.assertEqual([call.args[0] for call in stat.call_args_list],paths+paths)

    def test_historical_exception_authenticates_before_parse(self):
        p=self.root/'historical.json'; p.write_bytes(b'0'+b' '*87840)
        pin=guard.pin_file(p)
        with patch.object(guard,'HISTORICAL_MAP',p),patch.object(guard,'HISTORICAL_MAP_SHA',pin['sha256']):
            self.assertEqual(guard.historical_map(p),0)
            p.write_bytes(b'1'+b' '*87840)
            with self.assertRaises(ValueError): guard.historical_map(p)
            p.write_bytes(b'0'+b' '*87841)
            with self.assertRaises(ValueError): guard.historical_map(p)
        with self.assertRaises(ValueError): guard.historical_map(p)


class Builder(Fixture):
    def fixture(self):
        run=self.root/'seventh-voter'; run.mkdir()
        sources=[guard.pin_file(path) for path in guard.expected_source_paths()]
        tools=[guard.pin_file(Path(guard.__file__).resolve()),guard.pin_file(Path(__file__).resolve())]
        binaries=[]
        for kind in ['live','replay']:
            p=run/f'{kind}-vrf-seventh-voter-window-v1'; p.write_bytes(b'synthetic-no-execution'); binaries.append(guard.pin_file(p))
        report=guard.read_json(guard.R3/'sixth-voter/vrf-sixth-voter-window-preflight.json',guard.FREEZE_CAP)
        prepared=guard.read_json(guard.PREPARATION)
        report['profile']=guard.PROFILE
        report['selection']=[dict(parent_manifest_index=r['original_parent_manifest_index'],source_row=r['source_row'],
             tuple=dict(identity=r['identity'],address_hex=r['account_hex'],logical_key_hex=guard.KEY,key_preimage_hex=f'0004{guard.VOTER}')) for r in prepared['rows']]
        report['authority']['prior_first_d']['selected']=dict(proof_index=64,identity=prepared['prior_first_d']['identity'],voter_hex=guard.VOTER,
             value_hex=prepared['prior_first_d']['accepted_value']['value_hex'],usable_vrf_key=True)
        report['authority']['six_input_owner']['executable']=binaries[0]
        report['vrf_sources'][-3]={'path':'vrf_shared_window.rs',**{k:v for k,v in guard.pin_file(guard.SRC/'vrf_shared_window.rs').items() if k!='path'}}
        for i,kind in enumerate(['live','replay']):
            path=f'bin/{kind}_vrf_seventh_voter_window.rs'
            report['vrf_sources'][-2+i]={'path':path,**{k:v for k,v in guard.pin_file(guard.SRC/path).items() if k!='path'}}
        for rows in [report['vrf_sources'],report['authority']['six_input_owner']['sources']]:
            for row in rows:
                if row['path'] in {'lib.rs',
                    '../../../../rust/crates/rustaxa-storage/src/lib.rs',
                    '../../../../rust/crates/rustaxa-storage/src/concrete_state/mod.rs',
                    '../../../../rust/crates/rustaxa-storage/src/concrete_state/witness.rs',
                    '../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs',
                    '../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_bundle.rs'}:
                    row.update({k:v for k,v in guard.pin_file((guard.SRC/row['path']).resolve()).items() if k!='path'})
        preflight=run/'preflight.json'; preflight.write_bytes(guard.encoded(report,guard.FREEZE_CAP))
        evidence=[]
        for name in ['common-authority-review.md','common-authority-amendment.md','code-acceptance-review.md','validation-summary.json','storage-validation-reuse.json',
                     'source-commit.json','source-commit.log','source-hook-summary.json','startup-reconciliation.json','instruction-successor-freeze.json']:
            p=self.root/name; p.write_text('synthetic fixture only'); evidence.append(guard.pin_file(p))
        for name in ['round3-closeout-freeze.json','frozen-sixth-source-manifest.json','sixth-voter/source-freeze.json',
                     'sixth-voter/vrf-sixth-voter-window-preflight.json','sixth-voter/family-final-review.md',
                     'seventh-voter/dependencies-closed.json','seventh-voter/builder-draft-dependencies-closed.json','seventh-voter-preparation.json']:
            evidence.append(guard.pin_file(guard.R3/name))
        spec=dict(schema=1,run=str(run),source_commit='0'*40,sources=sources,tool_sources=tools,binaries=binaries,
                  preflight=guard.pin_file(preflight),evidence=evidence,
                  copy_metadata=[dict(path=str(p),device=0,inode=0,mode=0,mtime_ns=0) for p in [guard.COPY,guard.COPY/'db/db',guard.COPY/'db/state_db']])
        path=run/'spec.json'; path.write_bytes(guard.encoded(spec,guard.METADATA_CAP))
        return path,spec,report

    def test_deterministic_builder_without_database_access(self):
        path,_,_=self.fixture()
        original=Path.stat
        def guarded_stat(p,*a,**kw):
            if p.is_relative_to(guard.COPY) or p.is_relative_to(guard.REPO/'data'): raise AssertionError('DB stat')
            return original(p,*a,**kw)
        with patch.object(Path,'stat',guarded_stat):
            one=guard.build(path); two=guard.build(path)
        self.assertEqual(one,two)
        self.assertFalse(json.loads(one)['accepted'])

    def test_missing_source_wrong_key_order_prior_and_caps(self):
        path,spec,report=self.fixture()
        altered=copy.deepcopy(spec); altered['sources'].pop(); path.write_bytes(guard.encoded(altered,guard.METADATA_CAP))
        with self.assertRaises(ValueError): guard.build(path)
        path.write_bytes(guard.encoded(spec,guard.METADATA_CAP))
        with patch.object(guard,'SOURCE_TOTAL',1):
            with self.assertRaises(ValueError): guard.build(path)
        with patch.object(guard,'PIN_COUNT',1):
            with self.assertRaises(ValueError): guard.build(path)
        with patch.object(guard,'PIN_TOTAL',1):
            with self.assertRaises(ValueError): guard.build(path)
        for mutate in [lambda r:r['selection'][0]['tuple'].update(logical_key_hex='0'*64),
                       lambda r:r['authority']['six_input_owner']['sources'].reverse(),
            lambda r:r['authority']['six_input_owner']['sources'][0].update(path='../src/parent_input_handoff.rs'),
            lambda r:r['selection'].reverse(),lambda r:r['selection'].pop(),
                       lambda r:r['authority']['prior_first_d']['selected'].update(proof_index=61),
                       lambda r:r['bounds']['storage_resources'][0].update(limit=65),
                       lambda r:r['selection'][0]['tuple']['identity'].update(state_root_hex='0'*64)]:
            bad=copy.deepcopy(report); mutate(bad)
            with self.assertRaises(ValueError): guard.validate_preflight(bad)


    def successor_fixture(self, ordinal):
        path,spec,report = self.fixture()
        fixed = next((profile,row) for profile,row in guard.SUCCESSORS.items() if row['ordinal']==ordinal)
        profile,row = fixed
        mapping=guard.read_json(guard.SHARED_RUN/'seventh-voter/vrf-seventh-voter-window-preflight.json',guard.FREEZE_CAP)
        # Synthetic request fixtures test binding; they grant no physical authority.
        raw=guard.read_json(guard.PREPARATION)['rows']
        indices=row['indices']
        report['profile']=profile
        prepared=dict(rows=[],prior_first_d=dict(
            identity=report['authority']['prior_first_d']['selected']['identity'],
            selection_index=row['prior_index'],parent_manifest_index=row['prior_index']*3+2,
            accepted_value=dict(value_hex='ab'*32)))
        for i,old in enumerate(raw):
            source=copy.deepcopy(old['source_row']); source['voter_hex']=row['voter']
            prepared['rows'].append(dict(original_parent_manifest_index=indices[i],source_row=source,
                                         identity=old['identity'],account_hex=old['account_hex']))
        report['selection']=[dict(parent_manifest_index=r['original_parent_manifest_index'],source_row=r['source_row'],
             tuple=dict(identity=r['identity'],address_hex=r['account_hex'],logical_key_hex=row['key'],
                        key_preimage_hex=f"0004{row['voter']}")) for r in prepared['rows']]
        report['authority']['prior_first_d']['selected']=dict(proof_index=51+row['prior_index'],
             identity=prepared['prior_first_d']['identity'],voter_hex=row['voter'],value_hex='ab'*32,usable_vrf_key=True)
        run=path.parent
        binaries=[]
        for kind in ['live','replay']:
            p=run/f'{kind}-vrf-{ordinal}-voter-window-v1'; p.write_bytes(b'synthetic-no-execution'); binaries.append(guard.pin_file(p))
        spec['binaries']=binaries
        report['authority']['six_input_owner']['executable']=binaries[0]
        for i,kind in enumerate(['live','replay']):
            source=f'bin/{kind}_vrf_{ordinal}_voter_window.rs'
            report['vrf_sources'][-2+i]={'path':source,**{k:v for k,v in guard.pin_file(guard.SRC/source).items() if k!='path'}}
        prep=run/f'vrf-{ordinal}-voter-preparation.json'
        prep.write_bytes(guard.encoded(prepared,guard.METADATA_CAP))
        spec['preparation']=guard.pin_file(prep)
        # Pin immutable history at original paths; current placeholder records are
        # synthetic evidence and can never make an independently reviewed READY.
        for name in ['common-authority-review.md','common-authority-amendment.md','code-acceptance-review.md',
                     'validation-summary.json','storage-validation-reuse.json','source-commit.json','source-commit.log',
                     'source-hook-summary.json','startup-reconciliation.json','instruction-successor-freeze.json',
                     'command-ready-review.md','seventh-voter/family-final-review.md','fact-outcome-freeze.json',
                     'final-closeout-freeze.json','source-freeze.json','seventh-voter/binary-freeze.json',
                     'seventh-voter/vrf-seventh-voter-window.rcv2','seventh-voter/vrf-seventh-voter-window-replay.json','seventh-voter/vrf-seventh-voter-window-preflight.json','seventh-voter/vrf-seventh-voter-window-command-freeze.json','seventh-voter/exact-commands.json']:
            spec['evidence'].append(guard.pin_file(guard.SHARED_RUN/name))
        for name in ['successor-authority-amendment.md','luna-map.md','luna-rows.json',
                     'source-successor-mapping.json','binary-successor-mapping.json','preparation-comparison.json']:
            p=self.root/name;p.write_text('synthetic fixture only');spec['evidence'].append(guard.pin_file(p))
        spec['evidence'].append(spec['preparation'])
        preflight=Path(spec['preflight']['path']);preflight.write_bytes(guard.encoded(report,guard.FREEZE_CAP))
        spec['preflight']=guard.pin_file(preflight)
        path.write_bytes(guard.encoded(spec,guard.METADATA_CAP))
        return path,spec,report,prepared

    def test_each_sealed_successor_builds_and_binds_outputs(self):
        for ordinal in ['eighth','ninth','tenth']:
            # Each fixture has its own isolated path and source-pinned spec.
            with self.subTest(ordinal=ordinal):
                case=Builder();case.setUp()
                try:
                    path,spec,report,_=case.successor_fixture(ordinal)
                    data=guard.build(path)
                    self.assertEqual(data,guard.build(path))
                    freeze=json.loads(data)
                    self.assertEqual(freeze['profile'],report['profile'])
                    self.assertEqual(freeze['source_count'],74)
                    self.assertIn(f'vrf-{ordinal}-voter-window.rcv2',freeze['capture_command'][2])
                finally:case.tearDown()

    def test_successor_cross_profile_preparation_root_key_order_and_sources(self):
        path,spec,report,prepared=self.successor_fixture('eighth')
        guard.build(path)
        preflight=Path(spec['preflight']['path'])
        mutations=[
            lambda r:r.update(profile='n4_remaining_vrf_voter_unknown_v1'),
            lambda r:r.update(profile=next(p for p,x in guard.SUCCESSORS.items() if x['ordinal']=='ninth')),
            lambda r:r['selection'][0]['tuple']['identity'].update(state_root_hex='0'*64),
            lambda r:r['selection'][0]['tuple'].update(logical_key_hex='0'*64),
            lambda r:r['authority']['six_input_owner']['sources'].reverse(),
            lambda r:r['authority']['six_input_owner']['sources'][0].update(path='../src/parent_input_handoff.rs'),
            lambda r:r['selection'].reverse(),
            lambda r:r['selection'].pop(),
            lambda r:r['selection'].__setitem__(1,copy.deepcopy(r['selection'][0])),
            lambda r:r['authority']['prior_first_d']['selected'].update(proof_index=64),
            lambda r:r['vrf_sources'][-1].update(path='bin/replay_vrf_ninth_voter_window.rs')]
        for mutate in mutations:
            changed=copy.deepcopy(report);mutate(changed)
            preflight.write_bytes(guard.encoded(changed,guard.FREEZE_CAP))
            altered=copy.deepcopy(spec);altered['preflight']=guard.pin_file(preflight)
            path.write_bytes(guard.encoded(altered,guard.METADATA_CAP))
            with self.assertRaises(ValueError):guard.build(path)
        preflight.write_bytes(guard.encoded(report,guard.FREEZE_CAP))
        path.write_bytes(guard.encoded(spec,guard.METADATA_CAP))
        prep=Path(spec['preparation']['path']);prep.write_bytes(guard.encoded(dict(prepared,rows=[]),guard.METADATA_CAP))
        with self.assertRaises(ValueError):guard.build(path)

    def test_successor_guard_consumes_own_marker_and_unknown_rejects(self):
        bounds=Bounds();bounds.root=self.root
        freeze,sha,value=bounds.trusted_freeze()
        for profile,row in guard.SUCCESSORS.items():
            value['profile']=profile
            freeze.write_bytes(guard.encoded(value,guard.FREEZE_CAP))
            sha=guard.pin_file(freeze)['sha256']
            with patch.object(guard,'verify_files',side_effect=ValueError('source drift')) as verify:
                with self.assertRaisesRegex(ValueError,'source drift'):guard.capture(freeze,sha)
                self.assertTrue((self.root/f"vrf-{row['ordinal']}-voter-window-attempt.json").is_file())
                with self.assertRaises(FileExistsError):guard.capture(freeze,sha)
                self.assertEqual(verify.call_count,1)
        value['profile']='unknown'
        freeze.write_bytes(guard.encoded(value,guard.FREEZE_CAP))
        with patch.object(guard,'exclusive',side_effect=AssertionError('unknown marker')):
            with self.assertRaises(ValueError):guard.capture(freeze,guard.pin_file(freeze)['sha256'])

class ResidualEncoding(Fixture):
    def encoded_pins(self, rows=None):
        return dict(prefixes=copy.deepcopy(guard.RESIDUAL_PREFIXES),
                    pins=rows if rows is not None else [[12,'gate.log',0,'0'*64]])

    def test_explicit_order_and_duplicate_charges(self):
        row=[12,'gate.log',0,'0'*64]
        value=self.encoded_pins([row,[12,'second.log',3,'a'*64],row])
        with patch.object(Path,'stat',side_effect=AssertionError('expansion must have no I/O')):
            pins=guard.expand_residual_evidence(value)
        self.assertEqual(len(pins),3)
        self.assertEqual(pins[0],pins[2])
        self.assertEqual(pins[1]['bytes'],3)
        self.assertEqual(pins[0]['path'],str(guard.RESIDUAL_RUN/'gate.log'))

    def test_fixed_prefix_table_and_shape(self):
        bad=[None,[],dict(prefixes=[],pins=[]),dict(self.encoded_pins(),extra=True)]
        for i in range(len(guard.RESIDUAL_PREFIXES)):
            v=self.encoded_pins();v['prefixes'][i]+='/';bad.append(v)
        for rows in [None,{},[{}],[[12,'gate.log',0]],[[12,'gate.log',0,'0'*64,1]]]:
            bad.append(self.encoded_pins(rows) if rows is not None else dict(prefixes=guard.RESIDUAL_PREFIXES,pins=None))
        with patch.object(Path,'stat',side_effect=AssertionError('invalid encoding inspected path')):
            for value in bad:
                with self.subTest(value=value), self.assertRaises(ValueError):guard.expand_residual_evidence(value)

    def test_typed_index_size_and_hash(self):
        rows=[]
        for index in [True,False,None,-1,14,0.0,'12']:rows.append([index,'gate.log',0,'0'*64])
        for size in [True,None,-1,guard.FILE_CAP+1,0.0,'0']:rows.append([12,'gate.log',size,'0'*64])
        for sha in [None,0,'0'*63,'0'*65,'A'*64,'z'*64]:rows.append([12,'gate.log',0,sha])
        for row in rows:
            with self.subTest(row=row),self.assertRaises(ValueError):guard.expand_residual_evidence(self.encoded_pins([row]))

    def test_lexical_and_database_rejection_before_access(self):
        suffixes=['','/absolute','tail/','a//b','.','..','a/../b','a/./b','a\\b','a\x00b','a\nb','a\x85b',
                  'AI.md','nested/AI.md','data/file','local/evm-state-db/snapshot-litenode-copy/file']
        with patch.object(Path,'stat',side_effect=AssertionError('stat before lexical rejection')), \
             patch.object(Path,'lstat',side_effect=AssertionError('lstat before lexical rejection')), \
             patch.object(Path,'open',side_effect=AssertionError('open before lexical rejection')):
            for suffix in suffixes:
                with self.subTest(suffix=suffix),self.assertRaises(ValueError):guard.expand_residual_evidence(self.encoded_pins([[0,suffix,0,'0'*64]]))

    def test_count_conflict_and_maximum_file_size(self):
        row=[12,'gate.log',0,'0'*64]
        self.assertEqual(len(guard.expand_residual_evidence(self.encoded_pins([row]*317))),317)
        with self.assertRaises(ValueError):guard.expand_residual_evidence(self.encoded_pins([row]*318))
        with self.assertRaises(ValueError):guard.expand_residual_evidence(self.encoded_pins([row,[12,'gate.log',1,'0'*64]]))
        self.assertEqual(guard.expand_residual_evidence(self.encoded_pins([[12,'large',guard.FILE_CAP,'f'*64]]))[0]['bytes'],guard.FILE_CAP)

    def test_residual_sealed_identity_and_guard_marker(self):
        fixed=guard.sealed_profile(guard.RESIDUAL_PROFILE)
        self.assertEqual(fixed['indices'],guard.RESIDUAL_INDICES)
        self.assertEqual(fixed['prior_index'],0)
        with self.assertRaises(ValueError):guard.sealed_profile('n4_residual_vrf_unknown_v1')
        run=self.root/'residual-first';run.mkdir()
        bounds=Bounds();bounds.root=run
        freeze,sha,value=bounds.trusted_freeze();value['profile']=guard.RESIDUAL_PROFILE
        freeze.write_bytes(guard.encoded(value,guard.FREEZE_CAP));sha=guard.pin_file(freeze)['sha256']
        with patch.object(guard,'RESIDUAL_RUN',self.root),patch.object(guard,'verify_files',side_effect=ValueError('residual offline drift')) as verify:
            with self.assertRaisesRegex(ValueError,'residual offline drift'):guard.capture(freeze,sha)
            self.assertTrue((run/'vrf-residual-first-voter-window-attempt.json').is_file())
            with self.assertRaises(FileExistsError):guard.capture(freeze,sha)
            self.assertEqual(verify.call_count,1)
        with patch.object(guard,'exclusive',side_effect=AssertionError('wrong run consumed marker')):
            with self.assertRaisesRegex(ValueError,'fixed residual guard run'):guard.capture(freeze,sha)

    def test_complete_profile_rejects_compressed_evidence(self):
        case=Builder();case.setUp()
        try:
            path,spec,_=case.fixture();spec['evidence']=self.encoded_pins()
            path.write_bytes(guard.encoded(spec,guard.METADATA_CAP))
            with self.assertRaisesRegex(ValueError,'complete profile evidence representation'):guard.build(path)
        finally:case.tearDown()

class ResidualBuilder(Fixture):
    def setUp(self):
        super().setUp()
        prefixes=copy.deepcopy(guard.RESIDUAL_PREFIXES);prefixes[12]=str(self.root)
        self.patch_run=patch.object(guard,'RESIDUAL_RUN',self.root);self.patch_run.start()
        self.patch_prefixes=patch.object(guard,'RESIDUAL_PREFIXES',prefixes);self.patch_prefixes.start()
        self.addCleanup(self.patch_run.stop);self.addCleanup(self.patch_prefixes.stop)

    def fixture(self):
        if not getattr(self, '_historical_instruction_patch', None):
            self._historical_instruction_patch = historical_instruction_fixture(self)
        # The bounded fresh offline authority report is data for synthetic
        # command fixtures. Placeholder acceptance records grant no READY.
        draft=Path('/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-authority-draft.json')
        report=guard.read_json(draft,guard.FREEZE_CAP)
        # Only the declared source successors change this saved authority fixture.
        # All selection, proof, value and authority fields stay intact.
        owned = {guard.SRC/'vrf_shared_window.rs'} | {
            guard.REPO/'rust/crates/rustaxa-storage/src'/name for name in (
                'lib.rs', 'concrete_state/mod.rs', 'concrete_state/witness.rs',
                'concrete_state/witness_live.rs', 'concrete_state/witness_bundle.rs')}
        # These five declared storage units gain the additive fixed45 API.
        # Authenticate current bytes; preserve every original assertion/value.
        for row in report['vrf_sources'] + report['authority']['six_input_owner']['sources']:
            actual = (guard.SRC/row['path']).resolve()
            if actual in owned:
                current = guard.pin_file(actual)
                row.update(bytes=current['bytes'], sha256=current['sha256'])
        run=self.root/'residual-first';run.mkdir()
        binaries=[]
        for kind in ['live','replay']:
            p=run/f'{kind}-vrf-residual-first-voter-window-v1';p.write_bytes(b'synthetic-no-execution');binaries.append(guard.pin_file(p))
        report['authority']['six_input_owner']['executable']=binaries[0]
        prior=report['authority']['prior_first_d']['selected']
        prepared=dict(schema=1,profile=guard.RESIDUAL_PROFILE,
            raw_authority_inputs=guard.read_json(guard.PREPARATION)['raw_authority_inputs'],
            rows=[dict(original_parent_manifest_index=r['parent_manifest_index'],source_row=r['source_row'],
                       identity=r['tuple']['identity'],account_hex=r['tuple']['address_hex']) for r in report['selection']],
            prior_first_d=dict(identity=prior['identity'],selection_index=0,parent_manifest_index=2,
                accepted_value=dict(value_hex=prior['value_hex'])))
        prep=run/'vrf-residual-first-voter-preparation.json';prep.write_bytes(guard.encoded(prepared,guard.METADATA_CAP))
        preflight=run/'preflight.json';preflight.write_bytes(guard.encoded(report,guard.FREEZE_CAP))
        old=guard.read_json(guard.PREVIOUS_RUN/'eighth-voter/instance-spec.json')
        mappings=guard.read_json(Path(guard.RESIDUAL_PREFIXES[13])/'manifest.json')['authorized_instruction_paths']
        replacements={x['path']:{k:x[k] for k in ['path','bytes','sha256']} for x in mappings}
        evidence=[replacements.get(x['path'],x) for x in old['evidence']]
        for name in ['code-acceptance-review.md','validation-summary.json','source-commit.json','source-commit.log',
                     'source-hook-summary.json','startup-reconciliation.json','instruction-successor-freeze.json',
                     'residual-authority-review.md','residual-metadata-encoding-review.md','sol-review/map-acceptance.md',
                     'luna-map/residual-424-corrected-v1.json','luna-map/provenance-corrected-v1.json',
                     'luna-map/integrity-corrected-v1.json','luna-map/build_residual_map_corrected_v1.py',
                     'source-successor-mapping.json','binary-successor-mapping.json','preparation-comparison.json',
                     'storage-validation-current.json','code-and-gates-freeze.json']:
            p=self.root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text('synthetic fixture, no acceptance');evidence.append(guard.pin_file(p))
        for name in [n+s for n in guard.RESIDUAL_GATES for s in ['.json','.log']]+[
            'storage/correction-test.json','storage/correction-test.log','source-freeze.json',
            'tool-source-freeze.json','residual-authority-draft.json']:
            p=self.root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text('synthetic gate, no acceptance');evidence.append(guard.pin_file(p))
        for name in ['3-n4_next_10point_plan.md','4-n6_restart_checkpoint.md',
                     '5-n6_remaining_branch_queue.md','6-next_executable_slice_prompt.md']:
            evidence.append(guard.pin_file(guard.PREVIOUS_RUN/'frozen-instructions'/name))
        evidence += [guard.pin_file(Path(guard.RESIDUAL_PREFIXES[13])/'manifest.json'),
                     guard.pin_file(guard.PREVIOUS_RUN/'eighth-voter/instance-spec.json'),guard.pin_file(prep)]
        rows=[]
        for pin in evidence:
            matches=[(i,prefix) for i,prefix in enumerate(guard.RESIDUAL_PREFIXES) if pin['path'].startswith(prefix+'/')]
            self.assertEqual(len(matches),1)
            i,prefix=matches[0];rows.append([i,pin['path'][len(prefix)+1:],pin['bytes'],pin['sha256']])
        spec=dict(schema=1,run=str(run),source_commit='0'*40,
            sources=[guard.pin_file(p) for p in guard.expected_source_paths(guard.RESIDUAL_PROFILE)],
            tool_sources=[guard.pin_file(Path(guard.__file__).resolve()),guard.pin_file(Path(__file__).resolve())],
            binaries=binaries,preflight=guard.pin_file(preflight),preparation=guard.pin_file(prep),
            evidence=dict(prefixes=copy.deepcopy(guard.RESIDUAL_PREFIXES),pins=rows),
            copy_metadata=[dict(path=str(p),device=0,inode=0,mode=0,mtime_ns=0) for p in [guard.COPY,guard.COPY/'db/db',guard.COPY/'db/state_db']])
        path=run/'spec.json';path.write_text(json.dumps(spec,separators=(',',':'))+'\n')
        return path,spec,report,prepared

    def test_positive_deterministic_expanded_freeze(self):
        path,spec,_,_=self.fixture()
        before=guard.expand_residual_evidence(spec['evidence'])
        one=guard.build(path);self.assertEqual(one,guard.build(path))
        result=json.loads(one)
        self.assertFalse(result['accepted']);self.assertEqual(result['source_count'],76)
        self.assertEqual(result['pins'][81:81+len(before)],before)
        self.assertEqual(result['pin_count'],len(result['pins']))
        self.assertEqual(result['pinned_file_bytes'],sum(x['bytes'] for x in result['pins']))
        self.assertTrue(result['capture_command'][2].endswith('.rcv3'))
        self.assertLessEqual(path.stat().st_size,65536);self.assertLessEqual(len(one),131072)

    def test_full_history_order_and_mandatory_occurrences(self):
        path,spec,_,_=self.fixture();guard.build(path)
        for mutate in [lambda v:v['evidence']['pins'].pop(0),
                       lambda v:v['evidence']['pins'].__setitem__(slice(0,2),v['evidence']['pins'][:2][::-1]),
                       lambda v:v['evidence']['pins'].__setitem__(0,copy.deepcopy(v['evidence']['pins'][1]))]:
            changed=copy.deepcopy(spec);mutate(changed);path.write_text(json.dumps(changed,separators=(',',':'))+'\n')
            with self.assertRaisesRegex(ValueError,'full inherited evidence sequence'):guard.build(path)
        changed=copy.deepcopy(spec);changed['evidence']['pins']=[x for x in changed['evidence']['pins'] if x[1]!='residual-authority-review.md']
        path.write_text(json.dumps(changed,separators=(',',':'))+'\n')
        with self.assertRaisesRegex(ValueError,'missing mandatory'):guard.build(path)

    def test_residual_root_key_prior_order_preparation_and_cli_substitution(self):
        path,spec,report,prepared=self.fixture();guard.build(path)
        preflight=Path(spec['preflight']['path'])
        mutations=[lambda r:r.update(profile=guard.PROFILE),
            lambda r:r['selection'][0]['tuple']['identity'].update(state_root_hex='0'*64),
            lambda r:r['selection'][0]['tuple'].update(logical_key_hex='0'*64),
            lambda r:r['selection'].reverse(),lambda r:r['selection'].pop(),
            lambda r:r['selection'].__setitem__(1,copy.deepcopy(r['selection'][0])),
            lambda r:r['authority']['prior_first_d']['selected'].update(proof_index=52),
            lambda r:r['authority']['prior_first_d']['selected'].update(value_hex='0'*64),
            lambda r:r['authority']['prior_first_d']['selected']['identity'].update(state_root_hex='0'*64),
            lambda r:r['bounds'].update(proofs_exact=98),
            lambda r:r['vrf_sources'][-1].update(path='bin/replay_vrf_tenth_voter_window.rs')]
        for mutate in mutations:
            changed=copy.deepcopy(report);mutate(changed);preflight.write_bytes(guard.encoded(changed,guard.FREEZE_CAP))
            altered=copy.deepcopy(spec);altered['preflight']=guard.pin_file(preflight);path.write_text(json.dumps(altered,separators=(',',':'))+'\n')
            with self.assertRaises(ValueError):guard.build(path)
        preflight.write_bytes(guard.encoded(report,guard.FREEZE_CAP));path.write_text(json.dumps(spec,separators=(',',':'))+'\n')
        prep=Path(spec['preparation']['path']);prep.write_bytes(guard.encoded(dict(prepared,rows=[]),guard.METADATA_CAP))
        with self.assertRaises(ValueError):guard.build(path)


class SecondResidualBuilder(Fixture):
    def setUp(self):
        # Keep historical records at their immutable paths; only new test-owned
        # paths move. The fixed prefix table remains the production table.
        self.original_run = guard.RESIDUAL_RUN
        self.temp = tempfile.TemporaryDirectory(prefix='second-fixture-', dir=self.original_run)
        self.root = Path(self.temp.name)
        tail = [str(self.root / Path(p).relative_to(self.original_run))
                if Path(p) not in [self.original_run/'code-and-gates-freeze.json',
                                  guard.FIRST_COMMAND_RECORD, guard.FIRST_OUTCOME_RECORD]
                and Path(p).is_relative_to(self.original_run) else p
                for p in guard.SECOND_EVIDENCE_TAIL]
        self.run_patch = patch.object(guard, 'RESIDUAL_RUN', self.root)
        self.tail_patch = patch.object(guard, 'SECOND_EVIDENCE_TAIL', tail)
        self.run_patch.start(); self.tail_patch.start()
        self.addCleanup(self.run_patch.stop); self.addCleanup(self.tail_patch.stop)

    def fixture(self):
        if not getattr(self, '_historical_instruction_patch', None):
            self._historical_instruction_patch = historical_instruction_fixture(self)
        report = guard.read_json(self.original_run/'residual-authority-draft.json',guard.FREEZE_CAP)
        candidate_path = self.original_run/'luna-next/b92a46-spec-candidate.json'
        guard.pin_file(candidate_path, dict(path=str(candidate_path),bytes=74979,
                       sha256='1bd797ee59d36855cd52fdf80109c984d90f3a67059af2dbfe63b80952f0483e'))
        candidate=json.loads(candidate_path.read_bytes())
        report['profile']=guard.SECOND_PROFILE
        report['selection']=[]
        for row in candidate['requests']:
            source=dict(delayed_period=row['delayed_period'],proof=None,qualified=False,
                        semantic_key=row['semantic_key'],source_references=row['source_references'],
                        value=None,voter_hex=row['voter_hex'])
            report['selection'].append(dict(parent_manifest_index=row['parent_manifest_index'],
                source_row=source,tuple=dict(identity=dict(period=row['delayed_period'],state_root_hex=row['state_root_hex']),
                address_hex=row['account_hex'],logical_key_hex=row['logical_key_hex'],key_preimage_hex=row['key_preimage_hex'])))
        prior=report['authority']['prior_first_d']['selected']
        prior.update(proof_index=63,voter_hex=guard.SECOND_VOTER,value_hex=candidate['prior_first_d']['value_hex'])
        report['vrf_sources'][-2]['path']='bin/live_vrf_residual_b92a.rs'
        report['vrf_sources'][-1]['path']='bin/replay_vrf_residual_b92a.rs'
        sources=[guard.pin_file(p) for p in guard.expected_source_paths(guard.SECOND_PROFILE)]
        source_map={p['path']:p for p in sources}
        for row in report['vrf_sources']+report['authority']['six_input_owner']['sources']:
            pin=source_map[str((guard.SRC/row['path']).resolve())]
            row.update(bytes=pin['bytes'],sha256=pin['sha256'])
        run=self.root/'residual-second';run.mkdir()
        binaries=[]
        for kind in ['live','replay']:
            p=run/f'{kind}-vrf-residual-second-voter-window-v1';p.write_bytes(b'synthetic-no-execution')
            binaries.append(guard.pin_file(p))
        report['authority']['six_input_owner']['executable']=binaries[0]
        prepared=dict(schema=1,profile=guard.SECOND_PROFILE,
            raw_authority_inputs=guard.read_json(guard.PREPARATION)['raw_authority_inputs'],
            rows=[dict(original_parent_manifest_index=r['parent_manifest_index'],source_row=r['source_row'],
                       identity=r['tuple']['identity'],account_hex=r['tuple']['address_hex']) for r in report['selection']],
            prior_first_d=dict(identity=prior['identity'],selection_index=12,parent_manifest_index=38,
                              accepted_value=dict(value_hex=prior['value_hex'])))
        prep=run/'vrf-residual-second-voter-preparation.json';prep.write_bytes(guard.encoded(prepared,guard.METADATA_CAP))
        preflight=run/'preflight.json';preflight.write_bytes(guard.encoded(report,guard.FREEZE_CAP))
        first=guard.read_json(guard.FIRST_COMMAND_RECORD,guard.FREEZE_CAP)
        # Use the authenticated first spec prefix, not a freeze-position guess.
        first_spec=guard.read_json(self.original_run/'residual-first/instance-spec.json')
        evidence=guard.expand_residual_evidence(first_spec['evidence'])[:233]
        for name in guard.SECOND_EVIDENCE_TAIL:
            p=Path(name)
            if p.is_relative_to(self.root):
                p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b'synthetic fixture, no acceptance')
            evidence.append(guard.pin_file(p))
        self.assertEqual(len(evidence),290)
        encoded=[]
        for pin in evidence:
            matches=[(i,p) for i,p in enumerate(guard.RESIDUAL_PREFIXES) if pin['path'].startswith(p+'/')]
            self.assertEqual(len(matches),1)
            i,p=matches[0];encoded.append([i,pin['path'][len(p)+1:],pin['bytes'],pin['sha256']])
        spec=dict(schema=1,run=str(run),source_commit='0'*40,sources=sources,
                  tool_sources=[guard.pin_file(guard.__file__),guard.pin_file(__file__)],binaries=binaries,
                  preflight=guard.pin_file(preflight),preparation=guard.pin_file(prep),
                  evidence=dict(prefixes=guard.RESIDUAL_PREFIXES,pins=encoded),
                  copy_metadata=[dict(path=str(p),device=0,inode=0,mode=0,mtime_ns=0)
                    for p in [guard.COPY,guard.COPY/'db/db',guard.COPY/'db/state_db']])
        path=run/'spec.json';path.write_text(json.dumps(spec,separators=(',',':'))+'\n')
        return path,spec,report,prepared

    def test_positive_second_exact375_preserves_first_sources(self):
        path,spec,_,_=self.fixture()
        result=guard.build(path);self.assertEqual(result,guard.build(path))
        freeze=json.loads(result)
        self.assertEqual(freeze['source_count'],78);self.assertEqual(freeze['pin_count'],375)
        self.assertEqual(freeze['pinned_file_bytes'],sum(r['bytes'] for r in freeze['pins']))
        self.assertFalse(freeze['accepted']);self.assertIn('.rcv3',freeze['capture_command'][2])
        self.assertTrue(any(r['path'].endswith('bin/live_vrf_residual_first.rs') for r in spec['sources']))

    def test_second_rejects_changed_sequence_and_cross_profile(self):
        path,spec,report,_=self.fixture();guard.build(path)
        for change in [lambda s:s['evidence']['pins'].pop(),
                       lambda s:s['evidence']['pins'].__setitem__(slice(233,235),s['evidence']['pins'][233:235][::-1]),
                       lambda s:s['sources'].pop(),lambda s:s['binaries'].reverse()]:
            bad=copy.deepcopy(spec);change(bad);path.write_text(json.dumps(bad,separators=(',',':'))+'\n')
            with self.assertRaises(ValueError):guard.build(path)
        for change in [lambda r:r.update(profile=guard.RESIDUAL_PROFILE),
                       lambda r:r['authority']['prior_first_d']['selected'].update(proof_index=51),
                       lambda r:r['selection'][35]['tuple']['identity'].update(period=25706930),
                       lambda r:r['selection'][0]['tuple'].update(logical_key_hex=guard.RESIDUAL_KEY),
                       lambda r:r['vrf_sources'][-1].update(path='bin/replay_vrf_residual_first.rs')]:
            bad=copy.deepcopy(report);change(bad)
            preflight=Path(spec['preflight']['path']);preflight.write_bytes(guard.encoded(bad,guard.FREEZE_CAP))
            altered=copy.deepcopy(spec);altered['preflight']=guard.pin_file(preflight)
            path.write_text(json.dumps(altered,separators=(',',':'))+'\n')
            with self.assertRaises(ValueError):guard.build(path)

    def test_second_rejects_same_size_forged_first_code_record(self):
        path,_,_,_=self.fixture();guard.build(path)
        original=guard.FIRST_CODE_RECORD.read_bytes()
        changed=bytearray(original);changed[-1]^=1
        forged=self.root/'forged-first-code.json';forged.write_bytes(changed)
        self.assertEqual(len(original),forged.stat().st_size)
        with patch.object(guard,'FIRST_CODE_RECORD',forged):
            with self.assertRaises(ValueError):guard.build(path)

    def test_second_strict_body_boundaries_charge_repeated_occurrences(self):
        binaries=[dict(bytes=220200960),dict(bytes=220200960)]
        other=dict(bytes=15728640)
        guard._validate_second_fit(binaries+[other,other],binaries)
        with self.assertRaisesRegex(ValueError,'strict second command fit'):
            guard._validate_second_fit(binaries+[other,other,dict(bytes=1)],binaries)
        too_large=[dict(bytes=220200961),dict(bytes=220200960)]
        with self.assertRaisesRegex(ValueError,'strict second command fit'):
            guard._validate_second_fit(too_large,too_large)
        guard._validate_second_fit([dict(bytes=31457280)],[])
        with self.assertRaisesRegex(ValueError,'strict second command fit'):
            guard._validate_second_fit([dict(bytes=31457281)],[])

    def test_second_marker_and_profile_specific_count_limit(self):
        fixed=guard.sealed_profile(guard.SECOND_PROFILE)
        self.assertEqual(fixed['prior_index'],12);self.assertEqual(fixed['indices'],guard.SECOND_INDICES)
        rows=[[12,'zero.log',0,'0'*64]]*315
        value=dict(prefixes=guard.RESIDUAL_PREFIXES,pins=rows)
        self.assertEqual(len(guard.expand_residual_evidence(value,guard.SECOND_PROFILE)),315)
        with self.assertRaises(ValueError):guard.expand_residual_evidence(dict(value,pins=rows+rows[:1]),guard.SECOND_PROFILE)
        with self.assertRaises(ValueError):guard.expand_residual_evidence(value,'unknown')
        run=self.root/'residual-second';run.mkdir();bounds=Bounds();bounds.root=run
        freeze,_,v=bounds.trusted_freeze();v['profile']=guard.SECOND_PROFILE
        freeze.write_bytes(guard.encoded(v,guard.FREEZE_CAP));sha=guard.pin_file(freeze)['sha256']
        with patch.object(guard,'verify_files',side_effect=ValueError('second offline drift')) as verify:
            with self.assertRaisesRegex(ValueError,'second offline drift'):guard.capture(freeze,sha)
            self.assertTrue((run/'vrf-residual-second-voter-window-attempt.json').is_file())
            with self.assertRaises(FileExistsError):guard.capture(freeze,sha)
            self.assertEqual(verify.call_count,1)

class ThirdResidualBuilder(Fixture):
    def setUp(self):
        self.original_run = guard.RESIDUAL_RUN
        self.temp = tempfile.TemporaryDirectory(prefix='third-fixture-', dir=self.original_run)
        self.root = Path(self.temp.name)
        immutable = {p['path'] for p in guard.SECOND_HISTORY_PINS}
        immutable |= {guard.THIRD_CORRECTION_PIN['path'], guard.THIRD_AUTHORITY_PIN['path']}
        tail = [str(self.root / Path(p).relative_to(self.original_run))
                if p not in immutable and Path(p).is_relative_to(self.original_run)
                and (Path(p).is_relative_to(self.original_run/'third-source')
                     or Path(p).is_relative_to(self.original_run/'residual-third')) else p
                for p in guard.THIRD_EVIDENCE_TAIL]
        self.run_patch = patch.object(guard, 'RESIDUAL_RUN', self.root)
        self.tail_patch = patch.object(guard, 'THIRD_EVIDENCE_TAIL', tail)
        self.run_patch.start(); self.tail_patch.start()
        self.addCleanup(self.run_patch.stop); self.addCleanup(self.tail_patch.stop)

    def fixture(self):
        if not getattr(self, '_historical_instruction_patch', None):
            self._historical_instruction_patch = historical_instruction_fixture(self)
        report = guard.read_json(self.original_run/'residual-authority-draft.json',guard.FREEZE_CAP)
        candidate_path = self.original_run/'luna-third/e50b46-candidate.json'
        guard.pin_file(candidate_path, dict(path=str(candidate_path),bytes=41913,
                       sha256='9f3fa64dda8af3daf4a11671b685c953daf294ed62f81b24183776f08f8a0dee'))
        candidate=json.loads(candidate_path.read_bytes())
        report['profile']=guard.THIRD_PROFILE
        report['selection']=[]
        for row in candidate['requests']:
            source=dict(delayed_period=row['delayed_period'],proof=None,qualified=False,
                        semantic_key=row['semantic_key'],source_references=row['source_references'],
                        value=None,voter_hex=row['voter_hex'])
            report['selection'].append(dict(parent_manifest_index=row['parent_manifest_index'],
                source_row=source,tuple=dict(identity=dict(period=row['delayed_period'],state_root_hex=row['state_root_hex']),
                address_hex=row['account_hex'],logical_key_hex=row['logical_key_hex'],key_preimage_hex=row['key_preimage_hex'])))
        prior=report['authority']['prior_first_d']['selected']
        prior.update(proof_index=67,voter_hex=guard.THIRD_VOTER,value_hex=candidate['prior_first_d']['value_hex'])
        report['vrf_sources'][-2]['path']='bin/live_vrf_residual_e50b.rs'
        report['vrf_sources'][-1]['path']='bin/replay_vrf_residual_e50b.rs'
        sources=[guard.pin_file(p) for p in guard.expected_source_paths(guard.THIRD_PROFILE)]
        source_map={p['path']:p for p in sources}
        for row in report['vrf_sources']+report['authority']['six_input_owner']['sources']:
            pin=source_map[str((guard.SRC/row['path']).resolve())]
            row.update(bytes=pin['bytes'],sha256=pin['sha256'])
        run=self.root/'residual-third';run.mkdir()
        binaries=[]
        for kind in ['live','replay']:
            p=run/f'{kind}-vrf-residual-third-voter-window-v1';p.write_bytes(b'synthetic-no-execution')
            binaries.append(guard.pin_file(p))
        report['authority']['six_input_owner']['executable']=binaries[0]
        prepared=dict(schema=1,profile=guard.THIRD_PROFILE,
            raw_authority_inputs=guard.read_json(guard.PREPARATION)['raw_authority_inputs'],
            rows=[dict(original_parent_manifest_index=r['parent_manifest_index'],source_row=r['source_row'],
                       identity=r['tuple']['identity'],account_hex=r['tuple']['address_hex']) for r in report['selection']],
            prior_first_d=dict(identity=prior['identity'],selection_index=16,parent_manifest_index=50,
                              accepted_value=dict(value_hex=prior['value_hex'])))
        prep=run/'vrf-residual-third-voter-preparation.json';prep.write_bytes(guard.encoded(prepared,guard.METADATA_CAP))
        preflight=run/'preflight.json';preflight.write_bytes(guard.encoded(report,guard.FREEZE_CAP))
        first=guard.read_json(guard.FIRST_COMMAND_RECORD,guard.FREEZE_CAP)
        # Use the authenticated first spec prefix, not a freeze-position guess.
        first_spec=guard.read_json(self.original_run/'residual-first/instance-spec.json')
        evidence=guard.expand_residual_evidence(first_spec['evidence'])[:233]
        for name in guard.THIRD_EVIDENCE_TAIL:
            p=Path(name)
            if p.is_relative_to(self.root):
                p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b'synthetic fixture, no acceptance')
            evidence.append(guard.pin_file(p))
        self.assertEqual(len(evidence),288)
        encoded=[]
        for pin in evidence:
            matches=[(i,p) for i,p in enumerate(guard.RESIDUAL_PREFIXES) if pin['path'].startswith(p+'/')]
            self.assertEqual(len(matches),1)
            i,p=matches[0];encoded.append([i,pin['path'][len(p)+1:],pin['bytes'],pin['sha256']])
        spec=dict(schema=1,run=str(run),source_commit='0'*40,sources=sources,
                  tool_sources=[guard.pin_file(guard.__file__),guard.pin_file(__file__)],binaries=binaries,
                  preflight=guard.pin_file(preflight),preparation=guard.pin_file(prep),
                  evidence=dict(prefixes=guard.RESIDUAL_PREFIXES,pins=encoded),
                  copy_metadata=[dict(path=str(p),device=0,inode=0,mode=0,mtime_ns=0)
                    for p in [guard.COPY,guard.COPY/'db/db',guard.COPY/'db/state_db']])
        path=run/'spec.json';path.write_text(json.dumps(spec,separators=(',',':'))+'\n')
        return path,spec,report,prepared

    def test_third_positive_exact375_keeps_all_four_old_cli_sources(self):
        path,spec,_,_=self.fixture()
        result=guard.build(path); self.assertEqual(result,guard.build(path))
        freeze=json.loads(result)
        self.assertEqual(freeze['source_count'],80); self.assertEqual(freeze['pin_count'],375)
        self.assertEqual(freeze['pinned_file_bytes'],sum(p['bytes'] for p in freeze['pins']))
        self.assertFalse(freeze['accepted'])
        for profile in ['first','b92a']:
            for kind in ['live','replay']:
                self.assertTrue(any(p['path'].endswith(f'bin/{kind}_vrf_residual_{profile}.rs')
                                    for p in spec['sources']))

    def test_third_requires_exact_fresh_gate_and_history_sequence(self):
        path,spec,_,_=self.fixture(); guard.build(path)
        for change in [lambda s:s['evidence']['pins'].pop(),
                       lambda s:s['evidence']['pins'].__setitem__(slice(233,235),s['evidence']['pins'][233:235][::-1]),
                       lambda s:s['sources'].pop(),lambda s:s['binaries'].reverse()]:
            bad=copy.deepcopy(spec); change(bad); path.write_text(json.dumps(bad,separators=(',',':'))+'\n')
            with self.assertRaises(ValueError): guard.build(path)
        # An old second gate is not a fresh third check even with correct bytes.
        bad=copy.deepcopy(spec)
        old=guard.pin_file(self.original_run/'residual-second/qualifier-all-package.json')
        index,prefix=next((i,p) for i,p in enumerate(guard.RESIDUAL_PREFIXES) if old['path'].startswith(p+'/'))
        bad['evidence']['pins'][233]=[index,old['path'][len(prefix)+1:],old['bytes'],old['sha256']]
        path.write_text(json.dumps(bad,separators=(',',':'))+'\n')
        with self.assertRaisesRegex(ValueError,'exact third evidence sequence'): guard.build(path)

    def test_third_rejects_prior_omission_key_and_cli_substitution(self):
        path,spec,report,_=self.fixture(); guard.build(path)
        for change in [lambda r:r.update(profile=guard.SECOND_PROFILE),
                       lambda r:r['authority']['prior_first_d']['selected'].update(proof_index=63),
                       lambda r:r['authority']['prior_first_d']['selected'].update(value_hex='00'*32),
                       lambda r:r['selection'][11]['tuple']['identity'].update(period=25706906),
                       lambda r:r['selection'][0]['tuple'].update(logical_key_hex=guard.SECOND_KEY),
                       lambda r:r['vrf_sources'][-1].update(path='bin/replay_vrf_residual_b92a.rs')]:
            bad=copy.deepcopy(report); change(bad)
            preflight=Path(spec['preflight']['path']); preflight.write_bytes(guard.encoded(bad,guard.FREEZE_CAP))
            altered=copy.deepcopy(spec); altered['preflight']=guard.pin_file(preflight)
            path.write_text(json.dumps(altered,separators=(',',':'))+'\n')
            with self.assertRaises(ValueError): guard.build(path)

    def test_third_rejects_forged_correction_and_history_records(self):
        path,spec,_,_=self.fixture(); guard.build(path)
        for constant in ['THIRD_CORRECTION_PIN','THIRD_AUTHORITY_PIN']:
            expected=getattr(guard,constant); data=bytearray(Path(expected['path']).read_bytes()); data[-1]^=1
            forged=self.root/(constant+'.json'); forged.write_bytes(data)
            with patch.object(guard,constant,dict(expected,path=str(forged))):
                with self.assertRaises(ValueError): guard.build(path)
        bad_history=copy.deepcopy(guard.SECOND_HISTORY_PINS)
        forged=self.root/'forged-second-code.json'
        data=bytearray(Path(bad_history[0]['path']).read_bytes()); data[-1]^=1; forged.write_bytes(data)
        bad_history[0]['path']=str(forged)
        with patch.object(guard,'SECOND_HISTORY_PINS',bad_history):
            with self.assertRaises(ValueError): guard.build(path)

    def test_third_caps_and_repeated_body_charges_fail_closed(self):
        pair=[dict(bytes=220200960),dict(bytes=220200960)]
        other=dict(bytes=15728640)
        guard._validate_third_fit(pair+[other,other],pair)
        with self.assertRaises(ValueError): guard._validate_third_fit(pair+[other,other,dict(bytes=1)],pair)
        pair=[dict(bytes=220200961),dict(bytes=220200959)]
        with self.assertRaisesRegex(ValueError,'strict third executable body cap'): guard._validate_third_fit(pair,pair)
        path,spec,_,_=self.fixture(); guard.build(path)
        evidence=guard.expand_residual_evidence(spec['evidence'],guard.THIRD_PROFILE)
        evidence[233]=dict(evidence[233],bytes=guard.THIRD_EVIDENCE_CAPS[0]+1)
        with self.assertRaisesRegex(ValueError,'strict third evidence body cap'): guard._validate_third_history(evidence)

    def test_third_single_use_marker_and_direct_count(self):
        rows=[[12,'zero.log',0,'0'*64]]*313
        value=dict(prefixes=guard.RESIDUAL_PREFIXES,pins=rows)
        self.assertEqual(len(guard.expand_residual_evidence(value,guard.THIRD_PROFILE)),313)
        with self.assertRaises(ValueError): guard.expand_residual_evidence(dict(value,pins=rows+rows[:1]),guard.THIRD_PROFILE)
        run=self.root/'residual-third'; run.mkdir(); bounds=Bounds(); bounds.root=run
        freeze,_,v=bounds.trusted_freeze(); v['profile']=guard.THIRD_PROFILE
        freeze.write_bytes(guard.encoded(v,guard.FREEZE_CAP)); sha=guard.pin_file(freeze)['sha256']
        with patch.object(guard,'verify_files',side_effect=ValueError('third offline drift')) as verify:
            with self.assertRaisesRegex(ValueError,'third offline drift'): guard.capture(freeze,sha)
            self.assertTrue((run/'vrf-residual-third-voter-window-attempt.json').is_file())
            with self.assertRaises(FileExistsError): guard.capture(freeze,sha)
            self.assertEqual(verify.call_count,1)


class FourthResidualControls(Fixture):
    """Bound the fixed fourth profile without opening a database or running proofs."""

    def preflight_fixture(self):
        root=guard.RESIDUAL_RUN
        candidate=guard.read_json(root/'fourth-map/45-candidate.json',guard.FREEZE_CAP)
        report=guard.read_json(root/'residual-authority-draft.json',guard.FREEZE_CAP)
        report['profile']=guard.FOURTH_PROFILE
        baseline=guard.read_json(guard.R3/'sixth-voter/vrf-sixth-voter-window-preflight.json',guard.FREEZE_CAP)
        report['bounds']=dict(baseline['bounds'],payload_tuples_exact=45,proofs_exact=96)
        report['selection']=[]
        for row in candidate['requests']:
            source=dict(delayed_period=row['delayed_period'],proof=None,qualified=False,
                semantic_key=row['semantic_key'],source_references=row['source_references'],
                value=None,voter_hex=row['voter_hex'])
            report['selection'].append(dict(parent_manifest_index=row['parent_manifest_index'],
                source_row=source,tuple=dict(identity=dict(period=row['delayed_period'],
                state_root_hex=row['state_root_hex']),address_hex=row['account_hex'],
                logical_key_hex=row['logical_key_hex'],key_preimage_hex=row['key_preimage_hex'])))
        prior=candidate['prior_first_d']
        report['authority']['prior_first_d']['selected']=dict(proof_index=prior['proof_index'],
            identity=prior['tuple']['identity'],voter_hex=candidate['voter_hex'],
            value_hex=prior['value_hex'],usable_vrf_key=True)
        prepared=dict(rows=[dict(original_parent_manifest_index=r['parent_manifest_index'],
            source_row=r['source_row'],identity=r['tuple']['identity'],account_hex=r['tuple']['address_hex'])
            for r in report['selection']],prior_first_d=dict(identity=prior['tuple']['identity'],
            selection_index=prior['selection_index'],parent_manifest_index=prior['parent_manifest_index'],
            accepted_value=dict(value_hex=prior['value_hex'])))
        path=self.root/'prepared.json';path.write_text(json.dumps(prepared))
        return report,prepared,path,candidate

    def test_fixed_45_selection_prior_and_complete_tuple_binding(self):
        report,prepared,path,candidate=self.preflight_fixture()
        guard.validate_preflight(report,path)
        self.assertEqual(len(report['selection']),45)
        self.assertEqual(candidate['payload_count'],45)
        self.assertEqual(report['bounds']['proofs_exact'],96)
        self.assertEqual(prepared['prior_first_d']['selection_index'],5)
        self.assertEqual(prepared['prior_first_d']['parent_manifest_index'],17)
        self.assertEqual(len(guard.expected_source_paths(guard.FOURTH_PROFILE)),82)
        self.assertEqual(89+287,376)  # direct occurrences plus the fixed 233+54 evidence.
        for mutate in [lambda r:r['selection'].reverse(),
                       lambda r:r['selection'][0]['tuple']['identity'].update(state_root_hex='00'*32),
                       lambda r:r['selection'][0]['tuple'].update(logical_key_hex='00'*32),
                       lambda r:r['selection'][0]['source_row'].update(source_references=[]),
                       lambda r:r['selection'][11]['tuple']['identity'].update(period=25706906),
                       lambda r:r['authority']['six_input_owner']['sources'][0].update(path='unreviewed-owner-source.rs'),
                       lambda r:r['authority']['prior_first_d']['selected'].update(proof_index=55),
                       lambda r:r['authority']['prior_first_d']['selected'].update(value_hex='00'*32)]:
            bad=copy.deepcopy(report);mutate(bad)
            with self.subTest(mutation=mutate),self.assertRaises(ValueError):guard.validate_preflight(bad,path)

    def test_fixed_omission_order_and_rcv4_are_sealed(self):
        fixed=guard.sealed_profile(guard.FOURTH_PROFILE)
        self.assertEqual(fixed['indices'],guard.FOURTH_INDICES)
        self.assertEqual(fixed['prior_index'],5)
        self.assertEqual(guard.FOURTH_EVIDENCE_TAIL.__len__(),54)
        rows=[[12,'bounded.log',0,'0'*64]]*311
        self.assertEqual(len(guard.expand_residual_evidence(
            dict(prefixes=guard.RESIDUAL_PREFIXES,pins=rows),guard.FOURTH_PROFILE)),311)
        with self.assertRaises(ValueError):guard.expand_residual_evidence(
            dict(prefixes=guard.RESIDUAL_PREFIXES,pins=rows+rows[:1]),guard.FOURTH_PROFILE)
        self.assertEqual(fixed['ordinal'],'residual-fourth')
        self.assertEqual(guard.PIN_COUNT,400)

    def test_exact_history_tail_caps_and_named_map_representations(self):
        evidence=[dict(path=f'/synthetic/inherited-{i}',bytes=0,sha256='0'*64) for i in range(233)]
        required={x['path']:x for x in guard.FOURTH_HISTORY_PINS+[guard.FOURTH_SUCCESSOR_ENVELOPE_PIN,
            guard.FOURTH_AUTHORITY_PIN,guard.FOURTH_CORRECTION_PIN]}
        for i,path in enumerate(guard.FOURTH_EVIDENCE_TAIL):
            evidence.append(required.get(path,dict(path=path,bytes=0,sha256='0'*64)))
        command=dict(pin_count=375,pinned_file_bytes=419290696,pins=[None]*375)
        command['pins'][362]=guard.FOURTH_HISTORY_PINS[0]
        command['pins'][363]=dict(path=str(guard.RESIDUAL_RUN/'luna-map/build_residual_map_corrected_v1.py'),
            bytes=14706,sha256='7c35487e939e94156a1a0e7bd3f14dba846ccf1004e908cce89d0302ab255e07')
        command['pins'][364]=dict(path=str(guard.RESIDUAL_RUN/'luna-map/residual-424-corrected-v1.json'),
            bytes=437589,sha256='57d27bbd664c0188d96c0d3377a85432c2c2190f8763039c53f125a6ae520291')
        seal=dict(command_freeze=guard.FOURTH_HISTORY_PINS[1],total_occurrences_with_fixed_parent=399,
                  total_bytes_with_fixed_parent=420538339,source_commit=command['source_commit'] if 'source_commit' in command else '0'*40)
        command['source_commit']=seal['source_commit']
        def read(path,*args,**kwargs):
            return command if str(path)==guard.FOURTH_HISTORY_PINS[1]['path'] else seal
        with patch.object(guard,'pin_file',side_effect=lambda p,expected=None,cap=guard.FILE_CAP: expected or dict(path=str(p),bytes=0,sha256='0'*64)), \
             patch.object(guard,'read_json',side_effect=read),patch.object(guard,'_validate_fourth_envelope'):
            guard._validate_fourth_history(evidence)
            bad=copy.deepcopy(evidence);bad[234],bad[235]=bad[235],bad[234]
            with self.assertRaisesRegex(ValueError,'exact fourth evidence sequence'):guard._validate_fourth_history(bad)
            bad=copy.deepcopy(evidence);bad[233]['bytes']=guard.FOURTH_EVIDENCE_CAPS[0]+1
            with self.assertRaisesRegex(ValueError,'strict fourth evidence body cap'):guard._validate_fourth_history(bad)
            bad=copy.deepcopy(evidence);bad[233]=dict(path='/synthetic/replacement',bytes=0,sha256='0'*64)
            with self.assertRaisesRegex(ValueError,'exact fourth evidence sequence'):guard._validate_fourth_history(bad)

    def test_fourth_body_limits_preserve_pair_and_other_budgets(self):
        pair=[dict(bytes=220200960),dict(bytes=220200960)]
        other=dict(bytes=15728640)
        guard._validate_third_fit(pair+[other,other],pair)
        with self.assertRaises(ValueError):guard._validate_third_fit(pair+[other,other,dict(bytes=1)],pair)
        with self.assertRaisesRegex(ValueError,'strict third executable body cap'):
            guard._validate_third_fit([dict(bytes=1)],[dict(bytes=220200961),dict(bytes=220200959)])

    def test_fourth_attempt_marker_is_single_use_and_output_collision_preserves_bytes(self):
        run=self.root/'residual-fourth';run.mkdir()
        spec=run/'spec.json';spec.write_text('{}')
        spec_pin=guard.pin_file(spec)
        value=dict(schema=1,profile=guard.FOURTH_PROFILE,run=str(run),source_commit='0'*40,
            cwd=str(guard.REPO),environment=guard.ENV,capture_command=['/bin/false'],pins=[spec_pin],
            pin_count=1,pinned_file_bytes=spec_pin['bytes'],source_count=82,complete_source_bytes=0,
            embedded_source_bytes=0,copy_metadata=[],accepted=False)
        freeze=run/'command-freeze.json';freeze.write_bytes(guard.encoded(value,guard.FREEZE_CAP))
        sha=guard.pin_file(freeze)['sha256']
        output=run/'vrf-residual-fourth-voter-window.rcv4';output.write_bytes(b'preserve')
        with patch.object(guard,'RESIDUAL_RUN',self.root),patch.object(guard,'verify_files'), \
             patch.object(guard,'build',return_value=freeze.read_bytes()), \
             patch.object(guard.subprocess,'check_output',return_value=b'0'*40), \
             patch.object(Path,'cwd',return_value=guard.REPO):
            with self.assertRaisesRegex(ValueError,'single-use output collision'):
                guard.capture(freeze,sha)
            marker=run/'vrf-residual-fourth-voter-window-attempt.json'
            self.assertTrue(marker.is_file())
            with self.assertRaises(FileExistsError):guard.capture(freeze,sha)
            self.assertEqual(output.read_bytes(),b'preserve')

    def test_fixed_successor_retains_full_failures_and_rejects_outer_substitution(self):
        # This positive path authenticates all four bodies and the known seven.
        guard._validate_fourth_envelope()
        original=Path(guard.FOURTH_SUCCESSOR_ENVELOPE_PIN['path']).read_bytes()
        value=json.loads(original)
        self.assertEqual(len(value['members']),4)
        self.assertEqual(value['members'][1]['bytes'],82937)
        self.assertEqual(value['members'][2]['bytes'],159238)
        path=self.root/'successor.json'
        for mutate in [lambda v:v['members'].reverse(),
                       lambda v:v['members'].append(copy.deepcopy(v['members'][3])),
                       lambda v:v['members'][0].update(body_utf8=v['members'][0]['body_utf8']+'\n'),
                       lambda v:v.update(schema_id='unreviewed-successor')]:
            changed=copy.deepcopy(value);mutate(changed)
            path.write_bytes(guard.encoded(changed,1048576))
            with patch.object(guard,'FOURTH_SUCCESSOR_ENVELOPE_PIN',guard.pin_file(path)), \
                 patch.object(guard,'_validate_fourth_base_envelope') as base:
                with self.assertRaises(ValueError):guard._validate_fourth_envelope()
                base.assert_not_called()
        path.write_bytes(b' ' * 1048577)
        with patch.object(guard,'FOURTH_SUCCESSOR_ENVELOPE_PIN',guard.pin_file(path)):
            with self.assertRaises(ValueError):guard._validate_fourth_envelope()

    def test_fourth_envelope_rejects_duplicate_keys_reorder_wrong_hash_and_symlink(self):
        # Validate the authenticated parser against isolated synthetic members.
        bodies=[f'member-{i}'.encode() for i in range(7)]
        members=[]
        for i,body in enumerate(bodies):
            p=self.root/f'member-{i}.txt';p.write_bytes(body)
            members.append(dict(original_absolute_path=str(p),bytes=len(body),
                sha256=__import__('hashlib').sha256(body).hexdigest(),body_utf8=body.decode()))
        envelope=self.root/'history.json'
        def install(rows,expected=members,raw=None):
            data=raw if raw is not None else json.dumps(dict(schema_id='n4-fixed-45-finite-projection-history-v1',
                artifact_role='fixture',extraction_provenance='fixture',members=rows),separators=(',',':')).encode()
            envelope.write_bytes(data)
            pin=dict(path=str(envelope),bytes=len(data),sha256=__import__('hashlib').sha256(data).hexdigest())
            expected_pins=[dict(path=r['original_absolute_path'],bytes=r['bytes'],sha256=r['sha256']) for r in expected]
            return patch.object(guard,'FOURTH_ENVELOPE_PIN',pin),patch.object(guard,'FOURTH_ENVELOPE_MEMBERS',expected_pins)
        p1,p2=install(members)
        with p1,p2:guard._validate_fourth_base_envelope()
        duplicate=b'{"schema_id":"n4-fixed-45-finite-projection-history-v1","schema_id":"x"}'
        p1,p2=install(members,raw=duplicate)
        with p1,p2,self.assertRaises(ValueError):guard._validate_fourth_base_envelope()
        reordered=copy.deepcopy(members);reordered[0],reordered[1]=reordered[1],reordered[0]
        p1,p2=install(reordered,expected=members)
        with p1,p2,self.assertRaises(ValueError):guard._validate_fourth_base_envelope()
        forged=copy.deepcopy(members);forged[0]['body_utf8']='forged'
        p1,p2=install(forged,expected=members)
        with p1,p2,self.assertRaises(ValueError):guard._validate_fourth_base_envelope()
        alias=self.root/'alias';alias.symlink_to(self.root/'member-0.txt')
        symlinked=copy.deepcopy(members);symlinked[0]['original_absolute_path']=str(alias)
        p1,p2=install(symlinked,expected=symlinked)
        with p1,p2,self.assertRaises(ValueError):guard._validate_fourth_base_envelope()

if __name__ == '__main__':
    unittest.main()
