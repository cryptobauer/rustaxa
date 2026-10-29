import json
import os
from pathlib import Path
import sys
import unittest
from unittest import mock
import uuid
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import era_bound_command as bound

class SuccessOnlyUnitRetention(unittest.TestCase):
    def prepare(self):
        parent=Path(os.environ['RUN_ARTIFACT_DIR'])/('unit-contract-'+uuid.uuid4().hex)
        registration=parent.parent/'unit-contract-registrations.jsonl'
        with registration.open('a') as stream:
            stream.write(json.dumps({'new_root':str(parent),'registered_before_creation':True})+'\n');stream.flush();os.fsync(stream.fileno())
        parent.mkdir();(parent/'legacy-temps').mkdir()
        root=parent/'legacy-temps/case';root.mkdir();(root/'data').write_bytes(b'actual successful result')
        stat=parent.stat();identity=root.stat()
        manifest={'identity':'contract','argv':['actual-test-case'],'source_manifest':{'sha256':'bound-test-input'},'scratch_parent':str(parent),'scratch_parent_identity':[stat.st_dev,stat.st_ino],'unit_scratch_ledger':str(parent/'unit.jsonl')}
        rows=[{'root':str(root),'state':state,'identity':[identity.st_dev,identity.st_ino] if state!='registered_before_creation' else None,'launch':'contract','test':'case','panicking':False} for state in ['registered_before_creation','created','handles_closed']]
        Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows))
        bound.save(parent/'source-result.json',{'child_exit':0,'source_stable':True,'source_manifest':manifest['source_manifest'],'argv':manifest['argv']})
        return parent,root,manifest,rows
    def test_success_witness_precedes_disposal(self):
        parent,root,manifest,_=self.prepare();bound.finish_unit_scratch(manifest)
        self.assertFalse(root.exists());self.assertTrue(list(parent.glob('unit-case-*.json')))
    def test_failure_and_source_mismatch_preserve(self):
        for result in [{'child_exit':101,'source_stable':True},{'child_exit':0,'source_stable':False}]:
            parent,root,manifest,_=self.prepare();(parent/'source-result.json').write_text(json.dumps(result))
            with self.assertRaises(AssertionError):bound.finish_unit_scratch(manifest)
            self.assertTrue(root.exists())
    def test_unknown_completion_preserves(self):
        parent,root,manifest,_=self.prepare();(parent/'source-result.json').unlink()
        with self.assertRaises(FileNotFoundError):bound.finish_unit_scratch(manifest)
        self.assertTrue(root.exists())
    def test_missing_closure_or_creation_preserves(self):
        for missing in ['created','handles_closed']:
            _,root,manifest,rows=self.prepare();Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows if row['state']!=missing))
            bound.finish_unit_scratch(manifest);self.assertTrue(root.exists())
    def test_unregistered_preserves(self):
        _,root,manifest,rows=self.prepare();Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows if row['state']!='registered_before_creation'))
        bound.finish_unit_scratch(manifest);self.assertTrue(root.exists())
    def test_panic_preserves(self):
        _,root,manifest,rows=self.prepare();rows[-1]['panicking']=True;Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows))
        bound.finish_unit_scratch(manifest);self.assertTrue(root.exists())
    def test_missing_witness_preserves(self):
        _,root,manifest,_=self.prepare()
        with mock.patch.object(bound,'save',side_effect=OSError('injected witness failure')):
            with self.assertRaises(OSError):bound.finish_unit_scratch(manifest)
        self.assertTrue(root.exists())
    def test_symlink_preserves(self):
        parent,root,manifest,_=self.prepare();root.rename(parent/'original');root.symlink_to(parent/'original',target_is_directory=True)
        with self.assertRaises(AssertionError):bound.finish_unit_scratch(manifest)
        self.assertTrue(root.is_symlink() and (parent/'original/data').exists())
    def test_root_replacement_preserves(self):
        parent,root,manifest,_=self.prepare();root.rename(parent/'original');root.mkdir()
        with self.assertRaises(AssertionError):bound.finish_unit_scratch(manifest)
        self.assertTrue(root.exists() and (parent/'original/data').exists())
    def test_parent_replacement_preserves(self):
        parent,root,manifest,_=self.prepare();saved=parent.with_name(parent.name+'-saved');parent.rename(saved);parent.mkdir();(parent/'legacy-temps').mkdir();(saved/'legacy-temps/case').rename(root)
        Path(manifest['unit_scratch_ledger']).write_bytes((saved/'unit.jsonl').read_bytes());(parent/'source-result.json').write_bytes((saved/'source-result.json').read_bytes())
        with self.assertRaises(AssertionError):bound.finish_unit_scratch(manifest)
        self.assertTrue(root.exists())
    def test_unrelated_and_escape_preserve(self):
        parent,root,manifest,rows=self.prepare();unrelated=parent/'unrelated';unrelated.mkdir()
        for row in rows:row['root']=str(unrelated)
        Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows))
        with self.assertRaises(AssertionError):bound.finish_unit_scratch(manifest)
        self.assertTrue(root.exists() and unrelated.exists())

    def test_whole_command_scope_requires_terminal_evidence(self):
        parent,root,manifest,rows=self.prepare()
        rows=rows[:-1];rows[0]['completion_scope']='whole_source_stable_command'
        Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows))
        bound.finish_unit_scratch(manifest)
        self.assertTrue(root.exists())
    def test_failed_panic_record_taint_preserves(self):
        _,root,manifest,rows=self.prepare();rows[-1]['tainted']=True
        Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows))
        bound.finish_unit_scratch(manifest);self.assertTrue(root.exists())
    def test_disposal_request_waits_for_whole_child_success(self):
        _,root,manifest,rows=self.prepare();rows[-1]['state']='disposal_requested'
        Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows))
        bound.finish_unit_scratch(manifest);self.assertFalse(root.exists())
    def test_unfinished_dependency_preserves(self):
        _,root,manifest,rows=self.prepare();rows[-1]['dependencies']=1
        Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows))
        bound.finish_unit_scratch(manifest);self.assertTrue(root.exists())
    def test_prior_panic_cannot_be_overwritten_by_later_closure(self):
        _,root,manifest,rows=self.prepare();panic=dict(rows[-1],panicking=True)
        rows.insert(2,panic)
        Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows))
        bound.finish_unit_scratch(manifest);self.assertTrue(root.exists())
    def test_wrong_creation_launch_preserves(self):
        _,root,manifest,rows=self.prepare();rows[1]['launch']='wrong-launch'
        Path(manifest['unit_scratch_ledger']).write_text(''.join(json.dumps(row)+'\n' for row in rows))
        with self.assertRaises(AssertionError):bound.finish_unit_scratch(manifest)
        self.assertTrue(root.exists())

class SuccessorRunSelection(unittest.TestCase):
    def prepare_config(self):
        run=Path(os.environ['RUN_ARTIFACT_DIR'])/('selection-'+uuid.uuid4().hex);run.mkdir()
        authority=run/'authority.json';bound.save(authority,{'authorized_test':True})
        baseline=run/'baseline.json';bound.save(baseline,{'run':str(run),'corpus':str(bound.CORPUS),'baseline_actual_bytes':0,'files':[]})
        config={'schema':1,'run':str(run),'authority':{'path':str(authority),'sha256':bound.sha(authority.read_bytes())},'baseline':{'path':str(baseline),'sha256':bound.sha(baseline.read_bytes())},'baseline_actual_bytes':0,'retained_live_cap_bytes':1073741824}
        path=run/'config.json';bound.save(path,config);return path,config
    def setUp(self):
        self.previous=(bound.RUN,bound.RUN_CONFIG,bound.ACTIVE_RESOURCES)
        self.previous_envelope=bound.ACTIVE_ENVELOPE
    def tearDown(self):
        bound.RUN,bound.RUN_CONFIG,bound.ACTIVE_RESOURCES=self.previous
        bound.ACTIVE_ENVELOPE=self.previous_envelope
    def test_new_run_has_zero_prior_charge_and_exact_cap(self):
        path,config=self.prepare_config();bound.configure_run(path)
        self.assertEqual(bound.RUN,Path(config['run']))
        self.assertEqual(bound.resources()['prior_charged_bytes'],0)
        self.assertEqual(bound.resources()['retained_live_cap_bytes'],1073741824)
    def test_old_run_or_wrong_cap_or_changed_authority_rejected(self):
        for mutation in ['old','cap','authority','baseline']:
            path,config=self.prepare_config()
            if mutation=='old':config['run']=str(bound.OLD_RUN)
            elif mutation=='cap':config['retained_live_cap_bytes']=1342177280
            else:config[mutation]['sha256']='invalid'
            path.write_text(json.dumps(config))
            with self.assertRaises(AssertionError):bound.configure_run(path)
            self.assertEqual((bound.RUN,bound.RUN_CONFIG,bound.ACTIVE_RESOURCES),self.previous)
    def test_config_outside_selected_run_rejected(self):
        path,config=self.prepare_config();outside=path.parent/'child';outside.mkdir();other=outside/'config.json';bound.save(other,config)
        with self.assertRaises(AssertionError):bound.configure_run(other)

class CutoverEnvelope(SuccessorRunSelection):
    def prepare_cutover(self):
        path,config=self.prepare_config()
        config.update(schema=2,authority=dict(bound.CUTOVER_AUTHORITY),
                      retained_live_cap_bytes=68719476736,aggregate_child_seconds=43200,
                      max_output_bytes=67108864,command_timeout_seconds=dict(bound.CUTOVER_TIMEOUTS))
        path.write_text(json.dumps(config))
        return path,config

    def test_explicit_cutover_authority_and_limits(self):
        path,_=self.prepare_cutover();bound.configure_run(path)
        self.assertEqual(bound.resources()['retained_live_cap_bytes'],68719476736)
        self.assertEqual(bound.resources()['prior_charged_bytes'],0)
        self.assertEqual(bound.ACTIVE_ENVELOPE['aggregate_child_seconds'],43200)
        for kind,limit in bound.CUTOVER_TIMEOUTS.items():
            bound.command_limits(limit,kind)
            with self.assertRaises(AssertionError):bound.command_limits(limit+0.01,kind)

    def test_cutover_invalid_numeric_caps_and_limits_rejected_atomically(self):
        invalid=[('retained_live_cap_bytes',value) for value in (True,68719476736.0,0,-1,float('nan'),float('inf'),68719476737)]
        invalid += [('aggregate_child_seconds',True),('aggregate_child_seconds',43200.0),('aggregate_child_seconds',43201),('max_output_bytes',True),('max_output_bytes',67108865),('command_timeout_seconds',dict(bound.CUTOVER_TIMEOUTS,integration=3601)),('command_timeout_seconds',dict(bound.CUTOVER_TIMEOUTS,focused=600.0)),('schema',True),('schema',3)]
        for key,value in invalid:
            with self.subTest(key=key,value=value):
                path,config=self.prepare_cutover();config[key]=value;path.write_text(json.dumps(config))
                with self.assertRaises(AssertionError):bound.configure_run(path)
                self.assertEqual((bound.RUN,bound.RUN_CONFIG,bound.ACTIVE_RESOURCES),self.previous)
                self.assertEqual(bound.ACTIVE_ENVELOPE,self.previous_envelope)

    def test_cutover_rejects_self_selected_authority_even_with_matching_hash(self):
        path,config=self.prepare_cutover()
        copy=path.parent/'copied-driver.txt';copy.write_bytes(Path(bound.CUTOVER_AUTHORITY['path']).read_bytes())
        config['authority']['path']=str(copy);path.write_text(json.dumps(config))
        with self.assertRaises(AssertionError):bound.configure_run(path)
        path,config=self.prepare_cutover();config['authority']['sha256']='0'*64;path.write_text(json.dumps(config))
        with self.assertRaises(AssertionError):bound.configure_run(path)

    def test_cutover_optional_scheduler_is_exact_and_symlinks_are_rejected(self):
        path,config=self.prepare_cutover()
        scheduler=path.parent/'scheduler.json';scheduler.write_bytes(b'{"immutable_scheduler":true}\n')
        config['scheduler_input']={'path':str(scheduler),'sha256':bound.sha(scheduler.read_bytes())}
        path.write_text(json.dumps(config));bound.configure_run(path)
        self.assertEqual(bound.ACTIVE_ENVELOPE['scheduler_input'],config['scheduler_input'])
        for replacement in ('wrong-hash','symlink','relative'):
            bad=dict(config);bad['scheduler_input']=dict(config['scheduler_input'])
            if replacement=='wrong-hash':bad['scheduler_input']['sha256']='0'*64
            elif replacement=='relative':bad['scheduler_input']['path']='scheduler.json'
            else:
                link=path.parent/'scheduler-link.json';link.symlink_to(scheduler);bad['scheduler_input']['path']=str(link)
            path.write_text(json.dumps(bad))
            with self.assertRaises(AssertionError):bound.configure_run(path)

    def test_cutover_requires_kind_and_finite_timeout_before_root_creation(self):
        path,_=self.prepare_cutover();bound.configure_run(path)
        for timeout,kind in [(600,None),(600,'unknown'),(True,'focused'),(0,'focused'),(-1,'focused'),(float('nan'),'focused'),(float('inf'),'integration')]:
            with self.subTest(timeout=timeout,kind=kind):
                with self.assertRaises(AssertionError):bound.command_limits(timeout,kind)
        with mock.patch.dict(os.environ,{'RUSTAXA_ERA_RUN_CONFIG':str(path)}),mock.patch.object(sys,'argv',['era_bound_command.py','--name','must-not-launch','--timeout','601','--command-kind','focused','--','unused']):
            with self.assertRaises(AssertionError):bound.main()
        self.assertFalse((bound.RUN/'transition-commands/must-not-launch').exists())

    def test_schema_one_and_default_keep_1800_second_limit(self):
        path,_=self.prepare_cutover();bound.configure_run(path)
        old,_=self.prepare_config();bound.configure_run(old)
        self.assertIsNone(bound.ACTIVE_ENVELOPE)
        bound.command_limits(1800)
        with self.assertRaises(AssertionError):bound.command_limits(1800.01)
        with self.assertRaises(AssertionError):bound.command_limits(3600,'integration')
        with mock.patch.object(bound,'RUN_CONFIG',None),mock.patch.object(bound,'ACTIVE_ENVELOPE',None):
            bound.command_limits(1800)
            with self.assertRaises(AssertionError):bound.command_limits(1801)

    def test_peak_exact_fit_and_one_byte_overflow(self):
        resource={'retained_live_cap_bytes':68719476736}
        output=1024;reserve=64*1024*1024
        current=resource['retained_live_cap_bytes']-reserve-output-16*1024*1024
        bound.admit_peak(resource,current,reserve,output)
        with self.assertRaises(AssertionError):bound.admit_peak(resource,current+1,reserve,output)
        for invalid in (True,0,-1,67108865,float('nan'),float('inf')):
            with self.assertRaises(AssertionError):bound.admit_peak(resource,0,0,invalid)

    def test_capture_combines_stdout_and_stderr_and_retains_overflow(self):
        directory=Path(os.environ['RUN_ARTIFACT_DIR'])/('cutover-overflow-'+uuid.uuid4().hex)
        command=[sys.executable,'-c','import os; os.write(1,b"o"*700); os.write(2,b"e"*700)']
        code,status=bound.capture(directory,command,timeout_seconds=5,max_output_bytes=1024)
        self.assertEqual(code,125)
        self.assertEqual(status['outcome'],'output_limit')
        self.assertFalse(status['capture_complete'])
        self.assertEqual((directory/'stdout').stat().st_size+(directory/'stderr').stat().st_size,1024)
        self.assertTrue((directory/'status.json').is_file())

class CargoTestBinaryBoundary(unittest.TestCase):
    def prepare(self):
        run=Path(os.environ['RUN_ARTIFACT_DIR'])/('runner-contract-'+uuid.uuid4().hex)
        directory=run/'transition-commands/case';directory.mkdir(parents=True)
        launch=directory/'launch.json';config={'path':str(run/'config.json'),'sha256':'test-config'}
        parent={'run_config':config,'cargo_test_runner':True,'environment':{'RUSTAXA_ERA_LAUNCH_MANIFEST':str(launch),'APPROVED_PUBLIC':'kept'},'source_manifest':{'path':'source.json','sha256':'source'},'extra_inputs':[],'toolchain_build':{},'declared_generated_inputs':[],'resources':{}}
        bound.save(launch,parent)
        executable=run/'selected-test';executable.write_text('#!/bin/sh\nexit 0\n');executable.chmod(0o700)
        return run,launch,config,parent,executable
    def test_only_approved_environment_and_cargo_runtime_paths_are_saved(self):
        with mock.patch.dict(os.environ,{'UNRELATED_SECRET_SENTINEL':'never-save','LD_LIBRARY_PATH':'cargo-runtime-path'}):
            environment=bound.test_binary_environment({'environment':{'APPROVED_PUBLIC':'kept'}},Path('/scratch'),Path('/launch'))
        self.assertNotIn('UNRELATED_SECRET_SENTINEL',environment)
        self.assertEqual(environment['LD_LIBRARY_PATH'],'cargo-runtime-path')
        self.assertEqual(environment['APPROVED_PUBLIC'],'kept')
    def test_exact_binary_arguments_and_success_boundary(self):
        for child_exit,names_stable,expected in [(0,True,0),(7,True,7),(0,False,125)]:
            run,launch,config,parent,executable=self.prepare()
            argv=[str(executable),'--test-threads=1','--nocapture']
            with mock.patch.object(bound,'RUN',run),mock.patch.object(bound,'RUN_CONFIG',config),mock.patch.dict(os.environ,{'RUSTAXA_ERA_LAUNCH_MANIFEST':str(launch),'UNRELATED_SECRET_SENTINEL':'never-save'}),mock.patch.object(bound,'verify_pinned_inputs') as verify,mock.patch.object(bound,'selected_names_match',return_value=names_stable),mock.patch.object(bound.subprocess,'run',return_value=bound.subprocess.CompletedProcess(argv,child_exit)) as execute,mock.patch.object(bound,'finish_unit_scratch') as finish:
                self.assertEqual(bound.test_binary_runner(argv),expected)
                self.assertEqual(execute.call_args.args[0],argv)
                self.assertEqual(verify.call_count,2)
                self.assertEqual(finish.call_count,int(expected==0))
            outcome=json.loads(next(launch.parent.glob('rust-test-binaries/*/source-result.json')).read_bytes())
            self.assertEqual(outcome['child_exit'],child_exit)
            self.assertEqual(outcome['source_stable'],names_stable)
            child=json.loads(next(launch.parent.glob('rust-test-binaries/*/launch.json')).read_bytes())
            self.assertNotIn('UNRELATED_SECRET_SENTINEL',child['environment'])
            self.assertEqual(child['source_manifest'],parent['source_manifest'])
            self.assertEqual(child['executable']['sha256'],bound.sha(executable.read_bytes()))
    def test_real_executable_and_known_source_mutation_retain_roots(self):
        for mutation in ('executable','source'):
            with self.subTest(mutation=mutation):
                run,launch,config,parent,executable=self.prepare()
                fixture=run/'isolated-source';fixture.mkdir()
                corpus=fixture/'corpus';corpus.mkdir()
                source=fixture/'rust/fixture.rs';source.parent.mkdir();source.write_text('const ORIGINAL: u8 = 1;\n')
                for relative in ('Makefile','AGENTS.md','doc/evm_research/n6_retained_commission_task_policy.md'):
                    path=fixture/relative;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('isolated runner source fixture\n')
                # This executable is task-owned and outside the source fixture.
                # Mutating it cannot alter the interpreter or acceptance sources.
                executable.write_text('#!'+sys.executable+'\n'+"""import json, os, sys
from pathlib import Path
launch=json.loads(Path(os.environ['RUSTAXA_ERA_LAUNCH_MANIFEST']).read_bytes())
root=Path(os.environ['TMPDIR'])/'retained-diagnostic'
ledger=Path(launch['unit_scratch_ledger'])
def event(state, identity):
    row={'root':str(root),'state':state,'identity':identity,'launch':launch['identity'],'source_manifest':launch['source_manifest'],'test':'real-isolated-mutation','panicking':False,'completion_scope':'whole_source_stable_command'}
    with ledger.open('a') as output:
        output.write(json.dumps(row)+'\\n');output.flush();os.fsync(output.fileno())
event('registered_before_creation',None)
root.mkdir();identity=[root.stat().st_dev,root.stat().st_ino]
event('created',identity)
(root/'diagnostic').write_bytes(b'preserve after actual zero-exit mutation')
event('disposal_requested',identity)
if sys.argv[1]=='executable':
    with Path(__file__).open('a') as output: output.write('# changed executable bytes\\n')
else:
    Path(sys.argv[2]).write_text('const MUTATED: u8 = 2;\\n')
""")
                executable.chmod(0o700)
                with mock.patch.object(bound,'ROOT',fixture),mock.patch.object(bound,'CORPUS',corpus):
                    inventory=[{'path':str(path.relative_to(fixture)),'bytes':path.stat().st_size,'sha256':bound.sha(path.read_bytes())} for path in bound.source_paths()]
                    source_manifest=run/'source.json';bound.save(source_manifest,inventory)
                    parent['source_manifest']={'path':str(source_manifest),'sha256':bound.sha(source_manifest.read_bytes())}
                    launch.write_text(json.dumps(parent))
                    with mock.patch.object(bound,'RUN',run),mock.patch.object(bound,'RUN_CONFIG',config),mock.patch.dict(os.environ,{'RUSTAXA_ERA_LAUNCH_MANIFEST':str(launch)}),mock.patch.object(bound,'settings',return_value={}):
                        code=bound.test_binary_runner([str(executable),mutation,str(source)])
                self.assertEqual(code,125)
                directory=next(launch.parent.glob('rust-test-binaries/*'))
                outcome=json.loads((directory/'source-result.json').read_bytes())
                self.assertEqual(outcome['child_exit'],0)
                self.assertFalse(outcome['source_stable'])
                root=directory/'scratch/legacy-temps/retained-diagnostic'
                self.assertEqual((root/'diagnostic').read_bytes(),b'preserve after actual zero-exit mutation')
                self.assertFalse(list(directory.glob('unit-case-*.json')))
                if mutation=='executable':
                    self.assertNotEqual(bound.sha(executable.read_bytes()),outcome['executable']['sha256'])
                else:
                    self.assertIn('MUTATED',source.read_text())

    def test_unknown_child_completion_never_disposes(self):
        run,launch,config,parent,executable=self.prepare()
        with mock.patch.object(bound,'RUN',run),mock.patch.object(bound,'RUN_CONFIG',config),mock.patch.dict(os.environ,{'RUSTAXA_ERA_LAUNCH_MANIFEST':str(launch)}),mock.patch.object(bound,'verify_pinned_inputs'),mock.patch.object(bound.subprocess,'run',side_effect=OSError('unknown child status')),mock.patch.object(bound,'finish_unit_scratch') as finish:
            with self.assertRaises(OSError):bound.test_binary_runner([str(executable)])
            finish.assert_not_called()
        self.assertFalse(list(launch.parent.glob('rust-test-binaries/*/source-result.json')))
    def test_added_selected_source_name_is_detected(self):
        run,launch,config,parent,executable=self.prepare()
        source=run/'source.json';bound.save(source,[{'path':'first.rs'}])
        with mock.patch.object(bound,'ROOT',run),mock.patch.object(bound,'source_paths',return_value=[run/'first.rs',run/'added.rs']):
            self.assertFalse(bound.selected_names_match({'source_manifest':{'path':str(source)},'declared_generated_inputs':[]}))

class SelectedOutputBound(unittest.TestCase):
    def test_selected_limit_matches_capture_admission_and_manifest(self):
        for selected in (128,67108864):
            run=Path(os.environ['RUN_ARTIFACT_DIR'])/('output-bound-'+uuid.uuid4().hex);run.mkdir()
            argv=['era_bound_command.py','--name','selected-limit']
            if selected!=67108864:argv+=['--max-output-bytes',str(selected)]
            argv+=['--',sys.executable,'-c','pass']
            resource={'run':str(run),'corpus':str(bound.CORPUS),'baseline_actual_bytes':0,'prior_charged_bytes':0,'retained_live_cap_bytes':1073741824}
            with mock.patch.object(bound,'RUN',run),mock.patch.object(bound,'RUN_CONFIG',None),mock.patch.object(bound,'configure_run'),mock.patch.object(bound,'snapshot',return_value=[]),mock.patch.object(bound,'settings',return_value={}),mock.patch.object(bound,'resources',return_value=resource),mock.patch.object(bound,'tree_bytes',return_value=0),mock.patch.object(sys,'argv',argv),mock.patch.object(bound,'capture',return_value=(0,{'child_exit':0,'capture_complete':True})) as capture:
                self.assertEqual(bound.main(),0)
            manifest=json.loads((run/'transition-commands/selected-limit/launch.json').read_bytes())
            self.assertEqual(manifest['max_output_bytes'],selected)
            self.assertEqual(manifest['peak_admission']['output_reserve_bytes'],selected)
            self.assertEqual(manifest['peak_admission']['failure_reserve'],selected)
            self.assertEqual(capture.call_args.kwargs['max_output_bytes'],selected)
    def test_invalid_limits_reject_before_any_launch(self):
        import contextlib,io
        for selected in ('0','-1','67108865','not-an-integer'):
            with mock.patch.object(sys,'argv',['era_bound_command.py','--max-output-bytes',selected]),mock.patch.object(bound,'capture') as capture,contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error:bound.main()
                self.assertEqual(error.exception.code,2);capture.assert_not_called()
    def test_actual_overflow_preserves_scratch_and_incomplete_raw_capture(self):
        owner=Path(os.environ['RUN_ARTIFACT_DIR'])/('output-overflow-'+uuid.uuid4().hex);owner.mkdir()
        root=owner/'failed-root';root.mkdir();(root/'diagnostic').write_bytes(b'preserve on output overflow')
        command=[sys.executable,'-c','import sys,time;sys.stdout.write("x"*4096);sys.stdout.flush();time.sleep(10)']
        code,status=bound.capture(owner/'capture',command,timeout_seconds=5,max_output_bytes=128)
        self.assertEqual(code,125);self.assertEqual(status['outcome'],'output_limit')
        self.assertFalse(status['capture_complete']);self.assertEqual(status['max_output_bytes'],128)
        self.assertEqual(sum(status['streams'][name]['bytes'] for name in ('stdout','stderr')),128)
        self.assertEqual((root/'diagnostic').read_bytes(),b'preserve on output overflow')
        self.assertTrue((owner/'capture/status.json').is_file())
        self.assertFalse((owner/'source-result.json').exists())

if __name__=='__main__':unittest.main()
