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


def historical_fifth_sources():
    """Authenticate all84 immutable A8a source bodies for legacy fixtures.

    New live source belongs to the sixth instance. This exact archived port
    keeps historical full-report and builder assertions on their accepted bytes.
    """
    manifest_pin = dict(path='/home/fry/artifacts/vrf-run-01a141ba-20261015/luna/historical-a8a-sources/manifest.json',
                        bytes=35002, sha256='b8f6b2d43a8ac5d40052c79f8c3a6c7e000a083b30ccbe2e75981d5e60cc41f1')
    guard.pin_file(manifest_pin['path'],manifest_pin,guard.METADATA_CAP)
    manifest = guard.read_json(manifest_pin['path'])
    source_pin = manifest['source_freeze']
    guard.pin_file(source_pin['path'],source_pin,guard.FREEZE_CAP)
    sources = guard.read_json(source_pin['path'],guard.FREEZE_CAP)
    if len(sources) != 84 or [r['path'] for r in sources] != guard.expected_source_paths(guard.FIFTH_PROFILE):
        raise ValueError('historical fifth exact source roster')
    if len(manifest['source_map']) != len(sources):
        raise ValueError('historical fifth body count')
    mapping = {}
    for source, body in zip(sources,manifest['source_map']):
        if dict(path=body['logical_path'],bytes=body['bytes'],sha256=body['sha256']) != source:
            raise ValueError('historical fifth body identity')
        guard.pin_file(body['physical_path'],dict(source,path=body['physical_path']),guard.SOURCE_CAP)
        mapping[source['path']] = Path(body['physical_path'])
    return sources,mapping


def historical_fifth_reference_fixture(test,contract,virtual):
    """Read exactly five accepted predecessor instruction bodies in old fixtures.

    Preserve logical roles and all hash, descriptor and byte checks. Both record
    builders and freshly authenticated verifier modules use this test-only port.
    """
    manifest=guard.read_json('/home/fry/artifacts/vrf-196-preparation-20261015/manifest.json')
    refs=[r for p in contract['reference_placements'] for r in p['references']]
    mapping={}
    for item in manifest['authorized_instruction_paths']:
        matches=[r for r in refs if r['path']==item['path']]
        if len(matches)!=1:
            raise ValueError('historical fifth instruction occurrence')
        required=matches[0];previous=item['preparation_predecessor']
        if (required['bytes'],required['sha256'])!=(previous['bytes'],previous['sha256']):
            raise ValueError('historical fifth predecessor identity')
        guard.pin_file(previous['path'],previous,guard.METADATA_CAP)
        mapping[item['path']]=Path(previous['path'])
    if len(mapping)!=5:
        raise ValueError('historical fifth exact instruction count')
    virtual.update(mapping)
    def install(module):
        original=module.canonical_path
        def canonical(path):
            checked=original(path)
            return original(str(mapping[str(checked)])) if str(checked) in mapping else checked
        active=patch.object(module,'canonical_path',side_effect=canonical)
        active.start();test.addCleanup(active.stop)
        return module
    install(records)
    loader=guard.record_verifier
    def load(pin):
        return install(loader(pin))
    active=patch.object(guard,'record_verifier',side_effect=load)
    active.start();test.addCleanup(active.stop)


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
        placements = [dict(destination=str(review), members=[dict(role=f'failure-{i}', **guard.pin_file(body)) for i in range(20)]),
                      dict(destination=str(code_path), members=[dict(role=f'code-failure-{i}', **guard.pin_file(body)) for i in range(2)]),
                      dict(destination=str(closure), members=[dict(role=f'closure-failure-{i}', **guard.pin_file(body)) for i in range(7)])]
        original = self.root/'full-result-equivalence.json'
        original.write_text(json.dumps(dict(schema=1, full_gate_output='complete gate output',
                                           matched_tests=11, new_facts=0, complete_equal=True)))
        placements[1]['members'] += [dict(role='full-result-equivalence', **guard.pin_file(original)),
                                     dict(role='previous-code-freeze', **guard.pin_file(body)),
                                     dict(role='builder-first-failure', **guard.pin_file(body))]
        summary_path = self.root/'equivalence-summary.json'
        summary_path.write_text(json.dumps(guard.equivalence_summary(json.loads(original.read_bytes()))))
        contract = dict(reference_placements=placements, instruction_reference_rows=instructions,
                        equivalence_summary=dict(path=str(summary_path), cap_bytes=16384,
                                                 full_record_role='full-result-equivalence'),
                        pending_validation_rows=dict(placement=str(code_path), roles=roles, paths=[str(p) for p in paths], caps=[65536]*8))
        for placement in placements:
            expected = placement['members'] + (pending + instructions if placement['destination'] == str(code_path) else [])
            wrapper = records.decode(records.build(placement['destination'], contract_pin, '', expected))
            if placement['destination'] == str(code_path):
                wrapper = dict(record_tool_pins=pending, evidence_record=wrapper)
            Path(placement['destination']).write_text(json.dumps(wrapper))
        evidence = [guard.pin_file(Path(p['destination'])) for p in placements] + [guard.pin_file(summary_path)]
        return contract, contract_pin, evidence, body, code_path

    def run_verifier(self, contract, contract_pin, evidence):
        with patch.object(guard, 'RESIDUAL_RUN', self.root), \
             patch.object(guard, 'FOURTH_RECORD_CONTRACT', contract_pin), \
             patch.object(guard, 'fourth_record_contract', return_value=contract):
            return guard.verify_fourth_records(evidence)

    def test_shared_frozen_verifier_full_occurrences_and_charges(self):
        contract, pin, evidence, body, _ = self.fixture()
        with patch.object(guard, 'pin_file', wraps=guard.pin_file) as hashes:
            result = self.run_verifier(contract, pin, evidence)
        full = next(row for row in contract['reference_placements'][1]['members']
                    if row['role'] == 'full-result-equivalence')
        self.assertFalse(any(str(call.args[0]) == full['path'] for call in hashes.call_args_list))
        self.assertEqual(result['semantic_read_bytes'], full['bytes'])
        self.assertEqual(result['reference_count'], 61)
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

    def test_projection_keeps_all_semantic_fields_and_full_output_digest(self):
        full = dict(schema=1, full_gate_output='complete output\n', sources=[dict(hash='exact')],
                    selectors=['all-required'], matched_tests=11, new_facts=0)
        summary = guard.equivalence_summary(full)
        self.assertEqual(summary['projection'], {k: v for k, v in full.items() if k != 'full_gate_output'})
        guard.verify_equivalence_summary(summary, full)
        for mutate in [lambda v: v['projection'].update(matched_tests=10),
                       lambda v: v['projection'].update(new_facts=False),
                       lambda v: v['omitted_output'].update(sha256='0'*64),
                       lambda v: v.update(full_record_role='unreviewed'),
                       lambda v: v.update(accepted=True),
                       lambda v: v.update(extra='unconsumed')]:
            value = copy.deepcopy(summary)
            mutate(value)
            with self.assertRaises(ValueError):
                guard.verify_equivalence_summary(value, full)

    def test_summary_slot_tamper_rejects_even_with_its_new_pin(self):
        contract, pin, evidence, _, _ = self.fixture()
        summary_path = Path(contract['equivalence_summary']['path'])
        summary = json.loads(summary_path.read_bytes())
        summary['projection']['matched_tests'] = 10
        summary_path.write_text(json.dumps(summary))
        evidence = [guard.pin_file(Path(row['path'])) for row in evidence]
        with self.assertRaisesRegex(ValueError, 'exact full equivalence projection'):
            self.run_verifier(contract, pin, evidence)

    def test_full_result_role_binding_rejects(self):
        contract, pin, evidence, _, _ = self.fixture()
        contract['equivalence_summary']['full_record_role'] = 'missing-child'
        with self.assertRaisesRegex(ValueError, 'unique full equivalence body role'):
            self.run_verifier(contract, pin, evidence)

    def test_equivalence_projection_cap_and_full_output_type(self):
        with self.assertRaises(ValueError):
            guard.equivalence_summary(dict(full_gate_output=None))
        full = dict(full_gate_output='', semantic='x'*16384)
        with self.assertRaises(ValueError):
            guard.verify_equivalence_summary(guard.equivalence_summary(full), full)


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

class FifthFiniteControls(Fixture):
    def test_fifth_authority_admission_exact_byte_boundary(self):
        value=json.loads(Path(guard.FIFTH_CONTRACT['path']).read_bytes())
        path=self.root/'finite-authority.json'
        authority=next(r for r in value['command_inventory'] if r['group']=='current_tail54' and r['index']==40)
        authority['path']=str(path)
        original=json.dumps(value,separators=(',',':')).encode()
        for size in (131072,131073):
            path.write_bytes(original+b' '*(size-len(original)))
            pin=guard.pin_file(path)
            with patch.object(guard,'FIFTH_CONTRACT',pin):
                if size == 131072:
                    self.assertEqual(guard.fifth_contract(),json.loads(original))
                else:
                    with self.assertRaises(ValueError):guard.fifth_contract()
        authority['path']='/synthetic/stale-authority.json'
        path.write_text(json.dumps(value,separators=(',',':')))
        with patch.object(guard,'FIFTH_CONTRACT',guard.pin_file(path)):
            with self.assertRaisesRegex(ValueError,'authority role binding'):guard.fifth_contract()

    def test_finite_role_order_repeats_caps_and_immutable_binding(self):
        path = self.root/'body'; path.write_bytes(b'complete')
        pin = guard.pin_file(path)
        required = dict(pin,group='immutable',index=0,cap_bytes=8)
        guard.verify_finite_inventory([pin,pin],[required,required])
        for pins, roster in [([pin],[required,required]),
                             ([dict(pin,sha256='0'*64)],[required]),
                             ([pin],[dict(required,cap_bytes=7)]),
                             ([pin],[dict(required,path=str(self.root/'other'))])]:
            with self.assertRaises((ValueError,FileNotFoundError)):
                guard.verify_finite_inventory(pins,roster)
        pending=dict(required,pending=True)
        guard.verify_finite_inventory([pin],[pending])
        path.write_bytes(b'changed!')
        with self.assertRaises(ValueError):guard.verify_finite_inventory([pin],[pending])

    def test_fixed_compact_expansion_preserves_order_and_charges(self):
        contract=guard.fifth_contract()
        rows=[[r.get('cap_bytes',r.get('bytes')),'0'*64] for r in contract['command_inventory'][89:374]]
        expanded=guard.expand_fifth_evidence(rows,contract)
        self.assertEqual(len(expanded),285)
        self.assertEqual([r['path'] for r in expanded],[r['path'] for r in contract['command_inventory'][89:374]])
        self.assertEqual(sum(r['bytes'] for r in expanded),sum(r[0] for r in rows))
        for bad in [rows[:-1],rows+rows[:1],[[1,'wrong']]+rows[1:],[[True,'0'*64]]+rows[1:],[[rows[0][0]+1,'0'*64]]+rows[1:]]:
            with self.assertRaises(ValueError):guard.expand_fifth_evidence(bad,contract)

    def test_fifth_charges_extra_semantic_read_and_exact_cap(self):
        pins=[dict(bytes=0) for _ in range(376)]
        children=dict(referenced_bytes=965416,semantic_read_bytes=45182,source_bytes=0)
        self.assertEqual(guard.fifth_read_charges(pins,children,0),1010598)
        with patch.object(guard,'PIN_TOTAL',1010598):
            self.assertEqual(guard.fifth_read_charges(pins,children,0),1010598)
        with patch.object(guard,'PIN_TOTAL',1010597),self.assertRaises(ValueError):
            guard.fifth_read_charges(pins,children,0)

    def test_fifth_complete_reference_replay_and_full_failure_drift(self):
        contract=copy.deepcopy(guard.fifth_contract())
        historical_fifth_reference_fixture(self,contract,{})
        evidence=[]
        for i,placement in enumerate(contract['reference_placements']):
            destination=str(self.root/f'record-{i}.json');placement['destination']=destination
            raw=records.build(destination,placement['common_contract'],'offline regression fixture',placement['references'])
            Path(destination).write_bytes(raw);evidence.append(guard.pin_file(destination))
        definition=contract['correction_record48'];failures=[]
        for required in definition['failures']:
            guard.pin_file(required['path'],{k:required[k] for k in ('path','bytes','sha256')})
            failures.append(dict(required,body_utf8=Path(required['path']).read_text()))
        correction=dict(schema=definition['schema'],failures=failures,findings='offline regression fixture',accepted=False)
        path=self.root/'corrections.json';path.write_text(json.dumps(correction))
        for row in contract['command_inventory']:
            if row['group']=='current_tail54' and row['index']==48:row['path']=str(path)
        with patch.object(guard,'fifth_contract',return_value=contract),patch.object(guard,'verify_fifth_equivalence') as current:
            result=guard.verify_fifth_records(evidence)
            self.assertEqual(result['reference_count'],64)
            self.assertEqual(result['referenced_bytes'],987974)
            self.assertEqual(result['semantic_read_bytes'],45182)
            self.assertEqual(current.call_count,1)
            self.assertEqual(current.call_args.args[:2],(evidence,contract))
            correction['failures'][0]['body_utf8']='X'+correction['failures'][0]['body_utf8'][1:]
            path.write_text(json.dumps(correction))
            with self.assertRaises(ValueError):guard.verify_fifth_records(evidence)

    def test_current_comparator_full_reports_and_exact_commands(self):
        path=Path('/home/fry/artifacts/vrf-residual-20261014-current/source/native-first-full-v3.log')
        payload=json.loads(path.read_text())
        sources,_=historical_fifth_sources()
        self.assertTrue(guard.verify_fifth_comparison(payload,sources,payload['tool_source']))
        mutations=[]
        bad=copy.deepcopy(payload);bad['schema']=True;mutations.append(bad)
        bad=copy.deepcopy(payload);bad['pairs'][0]['after_body_utf8']='X'+bad['pairs'][0]['after_body_utf8'][1:];mutations.append(bad)
        bad=copy.deepcopy(payload);bad['pairs'][1]['mappings'][0]['field']='/unapproved';mutations.append(bad)
        bad=copy.deepcopy(payload);bad['pairs'].reverse();mutations.append(bad)
        bad=copy.deepcopy(payload);bad['pairs'][0]['mappings'][0]['successor']['sha256']='0'*64;mutations.append(bad)
        for bad in mutations:
            with self.assertRaises(ValueError):guard.verify_fifth_comparison(bad,sources,payload['tool_source'])
        command=['rtk','proxy',*guard.FIFTH_COMPARISON_ARGV]
        log=dict(path='fixed',bytes=1,sha256='f'*64)
        result=dict(command=command,log=log,exit_code=0)
        guard.verify_fifth_gate_result(result,log,command)
        for bad in [dict(result,command=['true']),dict(result,log=dict(log,sha256='0'*64)),dict(result,exit_code=1),dict(result,exit_code=False)]:
            with self.assertRaises(ValueError):guard.verify_fifth_gate_result(bad,log,command)

    def test_current_comparator_parser_limits(self):
        path=self.root/'grown';path.write_bytes(b'x'*9)
        self.assertEqual(guard.read_fifth_body(path,9),b'x'*9)
        with self.assertRaises(ValueError):guard.read_fifth_body(path,8)
        with self.assertRaises(ValueError):records.decode(b'['*20+b'0'+b']'*20,guard.FREEZE_CAP)
        with self.assertRaises(ValueError):records.decode(b'['+b'0,'*9000+b'0]',guard.FREEZE_CAP)
        with self.assertRaises(ValueError):records.bounded_shape({'pairs':['x'*2097152]})

    def test_complete_fifth_builder_with_virtual_pending_records(self):
        # Only pending validation files use a test filesystem port. Immutable
        # history, full prior/preflight, actual sources/binaries and the shared
        # body verifier are real. This fixture grants no READY or fact authority.
        contract=guard.fifth_contract();virtual={}; counter=0;original_pin=guard.pin_file;original_path=guard.regular_path
        inventory=contract['command_inventory'];tail={r['index']:r['path'] for r in inventory if r['group']=='current_tail54'}
        def put(path,raw):
            nonlocal counter
            target=self.root/f'pending-{counter}';counter+=1
            target.write_bytes(raw);virtual[path]=target
        def put_json(path,value):put(path,(json.dumps(value,indent=2,sort_keys=True)+'\n').encode())
        def pin(path,expected=None,cap=guard.FILE_CAP):
            logical=str(path);physical=virtual.get(logical,Path(path))
            # The real hash/size/cap checks still run on every physical file.
            physical_expected=None if expected is None else dict(expected,path=str(physical))
            if physical.stat().st_size>cap:raise ValueError(f'fixture role cap {logical}: {physical.stat().st_size}>{cap}')
            result=original_pin(physical,physical_expected,cap);return dict(result,path=logical)
        _,historical=historical_fifth_sources()
        virtual.update(historical)
        historical_fifth_reference_fixture(self,contract,virtual)
        sources=[pin(p) for p in guard.expected_source_paths(guard.FIFTH_PROFILE)]
        tools=[original_pin(Path(guard.__file__).resolve()),original_pin(Path(__file__).resolve())]
        for row in inventory:
            if row.get('pending') and not Path(row['path']).exists():put_json(row['path'],{})
        real=Path('/home/fry/artifacts/vrf-residual-20261014-current/source')
        put(tail[1],(real/'qualifier-all-package.log').read_bytes())
        package=json.loads((real/'qualifier-all-package.json').read_text());package['log']=pin(tail[1]);put_json(tail[0],package)
        put_json(tail[38],sources);put_json(tail[39],tools)
        for placement in contract['reference_placements']:
            put(placement['destination'],records.build(placement['destination'],placement['common_contract'],'unaccepted offline fixture',placement['references']))
        definition=contract['correction_record48']
        put_json(tail[48],dict(schema=definition['schema'],failures=[dict(r,body_utf8=Path(r['path']).read_text()) for r in definition['failures']],findings='unaccepted offline fixture',accepted=False))
        payload=json.loads(Path('/home/fry/artifacts/vrf-residual-20261014-current/source/native-first-full-v3.log').read_text());payload['tool_source']=tools[0]
        put_json(tail[21],payload)
        put_json(tail[20],dict(command=['rtk','proxy',*guard.FIFTH_COMPARISON_ARGV],exit_code=0,log=pin(tail[21])))
        put(tail[47],virtual[tail[1]].read_bytes())
        put_json(tail[46],dict(schema=1,source_freeze=pin(tail[38]),gate_results=[pin(tail[x]) for x in (0,20)],gate_logs=[pin(tail[x]) for x in (1,21,47)],required_exact_tests=guard.FIFTH_EQUIVALENCE_TESTS,matched_tests=7,all14_unchanged_profiles_equal=True,native_and_first_d_complete_equal=True,accepted=False))
        report=guard.read_json(guard.FIFTH_RUN/'preflight.json',guard.FREEZE_CAP)
        preparation=dict(rows=[dict(original_parent_manifest_index=r['parent_manifest_index'],source_row=r['source_row'],identity=r['tuple']['identity'],account_hex=r['tuple']['address_hex']) for r in report['selection']],prior_first_d=dict(identity=report['authority']['prior_first_d']['selected']['identity'],selection_index=11,parent_manifest_index=35,accepted_value=dict(value_hex=report['authority']['prior_first_d']['selected']['value_hex'])))
        put_json(str(guard.FIFTH_RUN/'preparation.json'),preparation)
        metadata=guard.read_json(guard.RESIDUAL_RUN/'residual-fourth/instance-spec.json')['copy_metadata']
        with patch.object(guard,'pin_file',side_effect=pin),patch.object(guard,'regular_path',side_effect=lambda path:original_path(virtual.get(str(path),Path(path)))):
            spec=guard.fifth_spec_from_roster('1'*40,metadata)
            spec_path=guard.FIFTH_RUN/'instance-spec.json';put_json(str(spec_path),spec)
            raw=guard.build(spec_path);freeze=json.loads(raw)
            self.assertEqual(freeze['pin_count'],376)
            self.assertEqual(freeze['source_count'],84)
            self.assertLessEqual(len(json.dumps(spec,indent=2,sort_keys=True).encode()),65536)
            self.assertEqual(freeze['pinned_file_bytes'],sum(r['bytes'] for r in freeze['pins'])+987974+45182)
            guard.verify_files(freeze)
            for scalar in (True,1.0):
                invalid=copy.deepcopy(spec);invalid['schema']=scalar
                with self.subTest(schema=scalar),self.assertRaises(ValueError):
                    guard.build_fifth(spec_path,invalid,report)
            summary=json.loads(virtual[tail[46]].read_text())
            summary['matched_tests']=7.0;put_json(tail[46],summary)
            with self.assertRaises(ValueError):
                guard.verify_fifth_equivalence(freeze['pins'][89:374],contract,records)
            summary['matched_tests']=7;put_json(tail[46],summary)
            bad=copy.deepcopy(freeze);bad['pinned_file_bytes']-=45182
            with self.assertRaises(ValueError):guard.verify_files(bad)

    def test_fifth_preflight_seals_45_rows_and_prior(self):
        report,prepared,path,_=FourthResidualControls.preflight_fixture(self)
        contract=guard.fifth_contract(); candidate=guard.read_json(contract['candidate']['path'])
        report['profile']=guard.FIFTH_PROFILE
        report['selection']=[]
        for row in candidate['tuples']:
            source=dict(delayed_period=row['delayed_period'],proof=None,qualified=False,semantic_key='fe_vrf_00_04_address',source_references=row['source_references'],value=None,voter_hex=row['voter_hex'])
            report['selection'].append(dict(parent_manifest_index=row['parent_manifest_index'],source_row=source,tuple=dict(identity=dict(period=row['delayed_period'],state_root_hex=row['state_root_hex']),address_hex=row['account_hex'],logical_key_hex=row['logical_key_hex'],key_preimage_hex=row['key_preimage_hex'])))
        prior=contract['selected_prior']
        identity=prior['selection']['tuple']['identity']; value=prior['value']['value_hex']
        report['authority']['prior_first_d']['selected']=dict(proof_index=62,identity=identity,voter_hex=prior['voter'],value_hex=value,usable_vrf_key=True)
        prepared=dict(rows=[dict(original_parent_manifest_index=r['parent_manifest_index'],source_row=r['source_row'],identity=r['tuple']['identity'],account_hex=r['tuple']['address_hex']) for r in report['selection']],prior_first_d=dict(identity=identity,selection_index=11,parent_manifest_index=35,accepted_value=dict(value_hex=value)))
        path.write_text(json.dumps(prepared));guard.validate_preflight(report,path)
        for field,value in [('state_root_hex','00'*32),('source_references',[{'period':25706902,'vote_hash_hex':'00'*32}])]:
            bad=copy.deepcopy(report); prep=copy.deepcopy(prepared)
            if field == 'state_root_hex':
                bad['selection'][0]['tuple']['identity'][field]=value
                prep['rows'][0]['identity'][field]=value
            else:
                bad['selection'][0]['source_row'][field]=value
                prep['rows'][0]['source_row'][field]=value
            path.write_text(json.dumps(prep))
            with self.assertRaises(ValueError):guard.validate_preflight(bad,path)
        path.write_text(json.dumps(prepared))
        bad=copy.deepcopy(report);bad['selection'].reverse()
        with self.assertRaises(ValueError):guard.validate_preflight(bad,path)
        bad=copy.deepcopy(report);bad['authority']['prior_first_d']['selected']['proof_index']=56
        with self.assertRaises(ValueError):guard.validate_preflight(bad,path)

class SixthFixedControls(Fixture):
    def test_inventory_and_record_decoder_share_one_authenticated_read(self):
        # Preserve the exact accepted V12 fixture and every original assertion.
        self.enterContext(patch.object(guard,'SIXTH_FAILURE_UTF8',[22406,'38fcbe33ac05dde604893d3bd8e4822c5de5f831d3541187bccfa444d43d3a43']))
        body=Path('/home/fry/artifacts/vrf-run-01a141ba-20261015/astra/staging-and-prior-first-failures.json').read_text()
        path=self.root/'source/validation-summary.json'
        path.parent.mkdir()
        path.write_text(json.dumps(dict(s7_failure_utf8=body)))
        pin=guard.pin_file(path)
        expected=[dict(group='current_tail54',index=29,path=str(path),cap_bytes=65536,pending=True)]
        original=Path.open;reads=[]
        def observe(target,*args,**kwargs):
            if target==path:reads.append(args)
            return original(target,*args,**kwargs)
        with patch.object(guard,'SIXTH_RUN',self.root/'fbb45'),patch.object(Path,'open',observe),patch.object(guard,'read_fifth_body',side_effect=AssertionError('unexpected reread')),patch.object(guard,'read_json',side_effect=AssertionError('unexpected nested read')):
            raw=guard.verify_finite_inventory([pin],expected,guard.SIXTH_PROFILE)
            guard.verify_sixth_summary_buffer(raw,pin,records)
            self.assertEqual(len(reads),1)
            # A retained buffer can be revalidated without reopening its path.
            guard.verify_finite_inventory([pin],expected,guard.SIXTH_PROFILE,raw)
            self.assertEqual(len(reads),1)
            for bad in [None,raw[:-1],b'X'+raw[1:]]:
                with self.assertRaises(ValueError):guard.verify_sixth_summary_buffer(bad,pin,records)
            self.assertEqual(len(reads),1)

    def test_complete_inline_failure_pin_and_all_mutations(self):
        # Preserve the exact accepted V12 fixture and every original assertion.
        self.enterContext(patch.object(guard,'SIXTH_FAILURE_UTF8',[22406,'38fcbe33ac05dde604893d3bd8e4822c5de5f831d3541187bccfa444d43d3a43']))
        body=Path('/home/fry/artifacts/vrf-run-01a141ba-20261015/astra/staging-and-prior-first-failures.json').read_text()
        guard.verify_sixth_summary(dict(s7_failure_utf8=body))
        for bad in [None,{},dict(s7_failure_utf8=None),dict(s7_failure_utf8=body[:-1]),dict(s7_failure_utf8='X'+body[1:]),dict(s7_failure_utf8=body+'x')]:
            with self.assertRaises(ValueError):guard.verify_sixth_summary(bad)

    def test_sixth_findings_reject_before_any_publication(self):
        contract=dict(correction_record48=dict(findings_utf8_cap=12288))
        with patch.object(guard,'finite_profile',return_value={}),patch.object(guard,'fifth_contract',return_value=contract),patch.object(guard,'exclusive') as output,patch.object(guard,'record_verifier') as verifier:
            for findings in ['x'*12289,'€'*4097,None]:
                with self.assertRaisesRegex(ValueError,'findings cap'):
                    guard.publish_fifth_records(findings,guard.SIXTH_PROFILE)
            output.assert_not_called()
            verifier.assert_not_called()

    def test_exact_fbb_identity_and_complete_source_closure(self):
        contract=guard.fifth_contract(guard.SIXTH_PROFILE)
        self.assertEqual(contract['s7_failure_utf8'],guard.SIXTH_FAILURE_UTF8)
        fixed=guard.sealed_profile(guard.SIXTH_PROFILE)
        self.assertEqual(fixed['voter'],'fbb3ee068e491e218d2cda595c23c6e3cc2f00f2')
        self.assertEqual(fixed['key'],'ec5c494ab92b4655da656eae4a546ee7efab77ffe269fb82adc3bf19c5d1ac73')
        self.assertEqual(fixed['prior_index'],19)
        self.assertEqual(len(fixed['indices']),45)
        self.assertEqual(len(set(fixed['indices'])),45)
        old=guard.expected_source_paths(guard.FIFTH_PROFILE)
        current=guard.expected_source_paths(guard.SIXTH_PROFILE)
        self.assertEqual(len(current),86)
        self.assertEqual(current[:84],old)
        self.assertEqual(current[84:],[str(guard.SRC/f'bin/{kind}_vrf_residual_fbb3.rs') for kind in ['live','replay']])

    def test_unknown_finite_config_and_unsettled_descriptor_reject(self):
        for profile in ['',guard.FOURTH_PROFILE,guard.SIXTH_PROFILE+'x']:
            with self.assertRaises(ValueError):guard.finite_profile(profile)
        for field in ['SIXTH_CONTRACT','SIXTH_PREDECESSOR_SOURCES','SIXTH_COMPARISON_PROFILES']:
            with patch.object(guard,field,None):
                with self.assertRaisesRegex(ValueError,'not settled'):guard.finite_profile(guard.SIXTH_PROFILE)

    def test_exact_sixth_compact_count_and_order(self):
        contract=json.loads(Path('/home/fry/artifacts/vrf-run-01a141ba-20261015/astra/fbb45-successor-contract-v4-candidate.json').read_bytes())
        expected=contract['command_inventory'][91:374]
        self.assertEqual(len(expected),283)
        pairs=[[r.get('bytes',0),r.get('sha256','0'*64)] for r in expected]
        rows=guard.expand_fifth_evidence(pairs,contract)
        self.assertEqual([r['path'] for r in rows],[r['path'] for r in expected])
        for bad in [pairs[:-1],pairs+[pairs[0]],[[True,'0'*64]]+pairs[1:],[[0,'invalid']]+pairs[1:]]:
            with self.assertRaises(ValueError):guard.expand_fifth_evidence(bad,contract)


class SixthInventoryControls(Fixture):
    def authority(self):
        return json.loads(Path(guard.SIXTH_CONTRACT['path']).read_bytes())

    def test_exact_expansion_preserves_all_roles_and_null_identity_without_io(self):
        original=self.authority(); value=copy.deepcopy(original)
        expected={name:[{k:v for k,v in zip(guard.SIXTH_INVENTORY_FIELDS,row) if v is not None}
                        for row in original[name]] for name in ['command_inventory','outcome_inventory']}
        command=value['command_inventory'];outcome=value['outcome_inventory']
        with patch.object(Path,'open',side_effect=AssertionError('decoder I/O')),patch.object(guard,'regular_path',side_effect=AssertionError('decoder resolve')):
            self.assertIs(guard.expand_sixth_inventory(value),value)
        self.assertIs(value['command_inventory'],command)
        self.assertIs(value['outcome_inventory'],outcome)
        for name in expected:self.assertEqual(value[name],expected[name])
        self.assertEqual(len(command)+len(outcome),400)
        self.assertEqual(command[0]['index'],0)
        self.assertNotIn('bytes',command[0])
        # The finite encoding preserves scalar identity; the sealed body still
        # owns actual role fields and cannot be supplied by a command caller.
        value=self.authority(); row=value['command_inventory'][91]
        self.assertIsNone(row[6]);row[6]=False;row[3]=0
        expanded=guard.expand_sixth_inventory(value)['command_inventory'][91]
        self.assertIs(expanded['pending'],False)
        self.assertIs(type(expanded['bytes']),int)
        self.assertEqual(expanded['bytes'],0)
        self.assertNotIn('cap_bytes',expanded)

    def test_header_counts_and_every_scalar_boundary_reject(self):
        def reject(change):
            value=self.authority();change(value)
            with self.assertRaises(ValueError):guard.expand_sixth_inventory(value)
        for key,bad in [('schema','other'),('fields',list(reversed(guard.SIXTH_INVENTORY_FIELDS))),('command_count',376.0),('outcome_count',23),('null_means_absent',1),('extra',0)]:
            with self.subTest(header=key):reject(lambda v:v['inventory_encoding'].__setitem__(key,bad))
        reject(lambda v:v.__setitem__('schema','n4-fbb45-finite-successor-v1'))
        for name in ['command_inventory','outcome_inventory']:
            reject(lambda v:v[name].pop())
            reject(lambda v:v[name].append(v[name][0]))
        for cell,bad in [(0,None),(1,True),(1,0.0),(1,-1),(2,'relative'),(2,'/a//b'),(2,'/a/../b'),(2,'/a/AI.md/b'),(2,'/a\\b'),(2,'/a'+chr(0)),(2,'/a/'+'b'*4096),(3,0),(4,'0'*64),(5,True),(5,-1),(6,1),(7,[]),(7,'unknown')]:
            with self.subTest(pending_cell=cell,bad=bad):reject(lambda v:v['command_inventory'][0].__setitem__(cell,bad))
        for cell,bad in [(3,None),(3,True),(3,-1),(4,None),(4,'G'*64),(4,'0'*63),(5,False),(5,0),(6,0),(6,True),(7,'s0')]:
            with self.subTest(immutable_cell=cell,bad=bad):reject(lambda v:v['command_inventory'][91].__setitem__(cell,bad))
        reject(lambda v:v['command_inventory'][0].append(None))
        reject(lambda v:v['command_inventory'].__setitem__(0,{}))

    def test_existing_node_depth_and_retained_caps_reject(self):
        for extra in [['x']*8192,[[[[[[[[[0]]]]]]]]],'x'*1769473,1.5]:
            value=self.authority();value['excess']=extra
            with self.assertRaises(ValueError):guard.expand_sixth_inventory(value)

    def test_sixth_maximum_correction_preserves_every_full_failure(self):
        contract=guard.fifth_contract(guard.SIXTH_PROFILE)
        prefix=Path('/home/fry/artifacts/vrf-round2-01a141ba-20261016/astra/v13-source-acceptance-retained-prefix.txt').read_text()
        findings=prefix+'x'*(12288-len(prefix.encode('utf-8')))
        published=[]
        with patch.object(guard,'record_verifier') as verifier,patch.object(guard,'exclusive',side_effect=lambda path,body:published.append((path,body))):
            guard.publish_fifth_records(findings,guard.SIXTH_PROFILE)
        self.assertEqual(len(published),1)
        path,body=published[0]
        self.assertEqual(str(path),next(r['path'] for r in contract['command_inventory'] if r['group']=='current_tail54' and r['index']==48))
        self.assertLessEqual(len(body),131072)
        value=json.loads(body)
        self.assertEqual(value['findings'],findings)
        self.assertIs(value['accepted'],False)
        self.assertEqual(len(value['failures']),31)
        for actual,required in zip(value['failures'],contract['correction_record48']['failures']):
            self.assertEqual({k:v for k,v in actual.items() if k!='body_utf8'},required)
            original=Path(required['path']).read_bytes()
            self.assertEqual(actual['body_utf8'].encode('utf-8'),original)
            self.assertEqual(len(original),required['bytes'])
            self.assertEqual(guard.hashlib.sha256(original).hexdigest(),required['sha256'])
        self.assertEqual(verifier.return_value.build.call_count,3)
        self.assertEqual(verifier.return_value.publish.call_count,3)

    def test_current_envelope_retains_complete_builder_failure_and_rejects_drift(self):
        path=Path('/home/fry/artifacts/vrf-round2-01a141ba-20261016/astra/actual-builder-and-prior-failures.json')
        body=path.read_text()
        guard.verify_sixth_summary(dict(s7_failure_utf8=body))
        self.assertEqual(guard.fifth_contract(guard.SIXTH_PROFILE)['s7_failure_utf8'],guard.SIXTH_FAILURE_UTF8)
        for bad in [None,body[:-1],'X'+body[1:],body+'x']:
            with self.assertRaises(ValueError):guard.verify_sixth_summary(dict(s7_failure_utf8=bad))



class SeventhHistoryControls(Fixture):
    def test_exact190_fixed_bytes_and_single_pass_bounded_reads(self):
        opened = []
        widths = []
        original = Path.open
        allowed = [p['path'] for p in guard.SEVENTH_HISTORY_PARTS]
        class Stream:
            def __init__(self, inner): self.inner = inner
            def __enter__(self): return self
            def __exit__(self, *args): self.inner.close()
            def read(self, width):
                if not 0 <= width <= 65536: raise AssertionError('unbounded read')
                widths.append(width)
                return self.inner.read(width)
        def observe(path, *args, **kwargs):
            self.assertIn(str(path), allowed)
            opened.append(str(path))
            return Stream(original(path, *args, **kwargs))
        with patch.object(Path, 'open', observe), patch.object(guard, 'pin_file', side_effect=AssertionError('second hash pass')), patch.object(guard, 'read_json', side_effect=AssertionError('recursive parse')):
            result = guard.verify_seventh_history()
        self.assertEqual(opened, allowed)
        self.assertEqual(result, dict(containers=5, members=190,
            charged_bytes=sum(p['bytes'] for p in guard.SEVENTH_HISTORY_PARTS),
            body_bytes=sum(r[3] for r in guard.SEVENTH_HISTORY_MEMBERS)))
        self.assertIn(65536, widths)

    def altered_part(self, raw):
        path = self.root/'altered-history.bin'
        path.write_bytes(raw)
        parts = copy.deepcopy(guard.SEVENTH_HISTORY_PARTS)
        parts[0].update(path=str(path), bytes=len(raw), sha256=guard.hashlib.sha256(raw).hexdigest())
        return patch.object(guard, 'SEVENTH_HISTORY_PARTS', parts)

    def test_corrupt_fixed_metadata_and_body_even_with_new_outer_hash(self):
        original = Path(guard.SEVENTH_HISTORY_PARTS[0]['path']).read_bytes()
        row = guard.SEVENTH_HISTORY_MEMBERS[0]
        body = 60 + len((row[0]+':'+str(row[1])).encode()) + len(row[2].encode())
        for offset in [0,8,10,12,16,18,20,28,60,body]:
            with self.subTest(offset=offset):
                bad = bytearray(original);bad[offset] ^= 1
                with self.altered_part(bad), self.assertRaises(ValueError):
                    guard.verify_seventh_history()

        parts = copy.deepcopy(guard.SEVENTH_HISTORY_PARTS)
        parts[0]['sha256'] = '0'*64
        with patch.object(guard,'SEVENTH_HISTORY_PARTS',parts),self.assertRaisesRegex(ValueError,'file hash'):
            guard.verify_seventh_history()

    def test_truncated_extra_and_invalid_utf8_are_rejected(self):
        original = Path(guard.SEVENTH_HISTORY_PARTS[0]['path']).read_bytes()
        invalid = bytearray(original);invalid[60] = 255
        for raw in [original[:-1],original+b'x',invalid]:
            with self.altered_part(raw), self.assertRaises(ValueError):
                guard.verify_seventh_history()

    def test_count_missing_duplicate_and_reorder_reject_without_prefix(self):
        for field in ['SEVENTH_HISTORY_PARTS','SEVENTH_HISTORY_MEMBERS']:
            original = copy.deepcopy(getattr(guard, field))
            variants = [original[:-1],original+[original[-1]]]
            changed = copy.deepcopy(original);changed[0],changed[1] = changed[1],changed[0];variants.append(changed)
            for value in variants:
                with patch.object(guard,field,value),self.assertRaises(ValueError):
                    guard.verify_seventh_history()

    def test_oversize_and_size_drift_reject_before_open(self):
        parts = copy.deepcopy(guard.SEVENTH_HISTORY_PARTS)
        for size in [guard.FILE_CAP+1,parts[0]['bytes']-1,True]:
            parts[0]['bytes'] = size
            with patch.object(guard,'SEVENTH_HISTORY_PARTS',parts),patch.object(Path,'open',side_effect=AssertionError('read before admission')),self.assertRaises(ValueError):
                guard.verify_seventh_history()

    def test_next42_remains_unsupported_by_command_builder(self):
        profile = 'n4_residual_vrf_7f821a7f63c0e7251e332b9b7c08da0b757124b5_v1'
        with self.assertRaises(ValueError): guard.finite_profile(profile)
        with self.assertRaises(ValueError): guard.sealed_profile(profile)


class SeventhReservationControls(Fixture):
    def test_exact_v7_role_caps_and_complete_future_reservation(self):
        authority = guard.read_json('/home/fry/artifacts/vrf-round3-01a141ba-20261016/astra/next42-successor-contract-v7.json',131072)
        rows = [r for r in authority['command_inventory'] if r[0]=='current_tail54']+[r for r in authority['command_inventory'] if r[0]=='precommand_paths']+authority['outcome_inventory']
        self.assertEqual(tuple(r[5] for r in rows), guard.SEVENTH_CURRENT_POOL_CAPS)
        future = sum(guard.SEVENTH_CURRENT_POOL_CAPS[57:])
        self.assertEqual(future,2523136)
        with patch.object(Path,'open',side_effect=AssertionError('unexpected I/O')):
            self.assertEqual(guard.reserve_seventh_current_evidence([0]*57+[None]*24),future)
            self.assertEqual(guard.reserve_seventh_current_evidence([0]*81),future)

    def test_exact_5mib_and_one_over_fail_before_io(self):
        sizes = [0]*57+[None]*24
        remaining = 5242880-sum(guard.SEVENTH_CURRENT_POOL_CAPS[57:])
        for index, cap in enumerate(guard.SEVENTH_CURRENT_POOL_CAPS[:57]):
            sizes[index] = min(cap,remaining)
            remaining -= sizes[index]
        self.assertEqual(remaining,0)
        with patch.object(Path,'open',side_effect=AssertionError('unexpected I/O')):
            self.assertEqual(guard.reserve_seventh_current_evidence(sizes),5242880)
            index = next(i for i,c in enumerate(guard.SEVENTH_CURRENT_POOL_CAPS[:57]) if sizes[i]<c)
            sizes[index] += 1
            with self.assertRaisesRegex(ValueError,'aggregate cap'):
                guard.reserve_seventh_current_evidence(sizes)

    def test_current_and_future_types_role_caps_and_missing_current(self):
        for index in [0,56,57,80]:
            for bad in [True,False,-1,1.0,'1',guard.SEVENTH_CURRENT_POOL_CAPS[index]+1]:
                sizes = [0]*57+[None]*24;sizes[index] = bad
                with self.subTest(index=index,bad=bad),patch.object(Path,'open',side_effect=AssertionError('unexpected I/O')),self.assertRaises(ValueError):
                    guard.reserve_seventh_current_evidence(sizes)
        sizes = [0]*57+[None]*24;sizes[56] = None
        with self.assertRaisesRegex(ValueError,'unresolved'):
            guard.reserve_seventh_current_evidence(sizes)

    def test_wrong_count_or_container_type_cannot_supply_prefix(self):
        for sizes in [[0]*80,[0]*82,tuple([0]*81),{},None]:
            with patch.object(Path,'open',side_effect=AssertionError('unexpected I/O')),self.assertRaises(ValueError):
                guard.reserve_seventh_current_evidence(sizes)

if __name__ == '__main__':
    unittest.main()
