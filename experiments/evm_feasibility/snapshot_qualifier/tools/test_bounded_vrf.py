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
                if row['path']=='lib.rs': row.update({k:v for k,v in guard.pin_file(guard.SRC/'lib.rs').items() if k!='path'})
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
                       lambda r:r['selection'].reverse(),lambda r:r['selection'].pop(),
                       lambda r:r['authority']['prior_first_d']['selected'].update(proof_index=61),
                       lambda r:r['bounds']['storage_resources'][0].update(limit=65),
                       lambda r:r['selection'][0]['tuple']['identity'].update(state_root_hex='0'*64)]:
            bad=copy.deepcopy(report); mutate(bad)
            with self.assertRaises(ValueError): guard.validate_preflight(bad)


if __name__ == '__main__':
    unittest.main()
