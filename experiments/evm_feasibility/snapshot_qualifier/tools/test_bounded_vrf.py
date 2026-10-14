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


class Fixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='vrf-tool-test-', dir='/home/fry/artifacts')
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()


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
        # The bounded fresh offline authority report is data for synthetic
        # command fixtures. Placeholder acceptance records grant no READY.
        draft=Path('/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-authority-draft.json')
        report=guard.read_json(draft,guard.FREEZE_CAP)
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

if __name__ == '__main__':
    unittest.main()
