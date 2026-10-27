#!/usr/bin/env python3
"""Source-at-launch binding for this finite transition closeout only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import shutil
from capture_command import capture

ROOT = Path(__file__).resolve().parents[1]
OLD_RUN = Path('/home/fry/artifacts/mainnet-era-20261026-01a179de')
RUN = OLD_RUN
RUN_CONFIG = None
ACTIVE_RESOURCES = None
CORPUS = ROOT / 'experiments/evm_feasibility/fixtures/mainnet_era'
# Original databases are not synthetic-gate source or input. No source selector
# descends into them; selected Rust/C++/Go/scripts/fixtures stay fully enumerated.
SOURCE_EXCLUDED_TREES = {ROOT/'data', ROOT/'local/evm-state-db/snapshot-litenode-copy'}
def sha(raw): return hashlib.sha256(raw).hexdigest()
def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
    descriptor=os.open(path.parent,os.O_RDONLY);os.fsync(descriptor);os.close(descriptor)
def configure_run(path):
    """Select only an explicitly pinned successor envelope; never inherit charges."""
    global RUN, RUN_CONFIG, ACTIVE_RESOURCES
    path=Path(path)
    assert path.is_absolute() and not path.is_symlink()
    path=path.resolve(strict=True)
    config=json.loads(path.read_bytes())
    run=Path(config['run'])
    assert config['schema']==1 and run.is_absolute() and run==run.resolve(strict=True)
    assert not run.is_relative_to(OLD_RUN) and run.is_relative_to(Path('/home/fry/artifacts'))
    assert path.parent==run and not path.is_symlink()
    for key in ('authority','baseline'):
        pin=config[key]; target=Path(pin['path'])
        assert target.is_absolute() and sha(target.read_bytes())==pin['sha256']
    assert config['retained_live_cap_bytes']==1073741824
    baseline=config['baseline_actual_bytes']
    assert type(baseline) is int and 0<=baseline<=tree_bytes(CORPUS)
    baseline_record=json.loads(Path(config['baseline']['path']).read_bytes())
    assert baseline_record['run']==str(run) and baseline_record['corpus']==str(CORPUS)
    assert baseline_record['baseline_actual_bytes']==baseline
    assert sum(row['bytes'] for row in baseline_record['files'])==baseline
    names=set()
    for row in baseline_record['files']:
        item=Path(row['path'])
        assert item.parent==CORPUS and not item.is_symlink() and item not in names
        assert item.stat().st_size==row['bytes'] and sha(item.read_bytes())==row['sha256']
        names.add(item)
    RUN=run; RUN_CONFIG={'path':str(path),'sha256':sha(path.read_bytes())}
    ACTIVE_RESOURCES={'run':str(run),'corpus':str(CORPUS),'baseline_actual_bytes':baseline,'prior_charged_bytes':0,'retained_live_cap_bytes':1073741824}

def resources():
    if ACTIVE_RESOURCES is not None: return dict(ACTIVE_RESOURCES)
    inventory=json.loads((RUN/'transition-admission-01/pre-authorization-inventory.json').read_bytes())
    recovery=json.loads(Path('/home/fry/artifacts/mainnet-era-session-review-20261027/source-manifest.json').read_bytes())
    baseline=sum(row['bytes'] for row in inventory if row['type']=='file')+sum(row['bytes'] for row in recovery['files'] if row['path'].startswith('experiments/evm_feasibility/fixtures/mainnet_era/'))
    return {'run':str(RUN),'corpus':str(CORPUS),'baseline_actual_bytes':baseline,'prior_charged_bytes':1024519292,'retained_live_cap_bytes':1342177280}

def source_paths(inputs=None, generated=()):
    paths = set()
    for parent, directories, files in os.walk(ROOT/'rust'):
        directories[:]=[name for name in directories if name not in ('.git','target','.cache','__pycache__')]
        for name in files:
            path=Path(parent)/name
            if path.suffix in ('.rs','.toml') or name in ('Cargo.lock','rust-toolchain','rust-toolchain.toml'):
                paths.add(path)
    for pattern in ('mainnet_era*','test_mainnet_era*','native_integration_reference*'):
        paths.update(p for p in (ROOT/'experiments/evm_feasibility').glob(pattern) if p.is_file())
    paths.update(p for p in (ROOT/'scripts').glob('*.py') if p.name in ('capture_command.py','era_bound_command.py','codex_quota.py'))
    paths.update([ROOT/'Makefile', ROOT/'AGENTS.md', ROOT/'doc/evm_research/n6_retained_commission_task_policy.md'])
    paths.update(p for p in (ROOT/'.cargo').glob('*') if p.is_file())
    for parent, directories, files in os.walk(ROOT):
        directories[:]=[name for name in directories if name not in ('.git','target','.cache','__pycache__') and Path(parent)/name not in SOURCE_EXCLUDED_TREES]
        for name in files:
            path=Path(parent)/name
            if path.name=='AI.md': continue
            if path.parent==CORPUS: continue
            if path.suffix in ('.cpp','.hpp','.h','.c','.cmake','.sh','.go','.py','.in','.json','.md','.txt','.yaml','.yml') or name in ('CMakeLists.txt','go.mod','go.sum','.gitignore','.gitmodules'):
                paths.add(path)
            if 'libraries/cli/include/cli/config_jsons/' in str(path) and path.suffix=='.json': paths.add(path)
            if '/fixtures/' in str(path) and path.parent!=CORPUS and path.suffix in ('.json','.gz','.rlp'):
                paths.add(path)
    paths.update(p for p in (inputs if inputs is not None else CORPUS.iterdir()) if p.is_file() and str(p) not in generated)
    return sorted(paths)
def snapshot(inputs=None, generated=()):
    index = {}
    for row in subprocess.check_output(['rtk','proxy','git','ls-files','-s','-z'],cwd=ROOT).split(b'\0'):
        if row:
            header,name=row.split(b'\t',1); index[name.decode()]=header.split()[1].decode()
    submodule=ROOT/'submodules/taraxa-evm'
    for row in subprocess.check_output(['rtk','proxy','git','ls-files','-s','-z'],cwd=submodule).split(b'\0'):
        if row:
            header,name=row.split(b'\t',1); index['submodules/taraxa-evm/'+name.decode()]=header.split()[1].decode()
    objects=RUN/'source-objects'; objects.mkdir(exist_ok=True)
    rows=[]
    for path in source_paths(inputs, generated):
        raw=path.read_bytes(); digest=sha(raw); relative=str(path.relative_to(ROOT))
        row={'path':relative,'bytes':len(raw),'sha256':digest}
        git_hash=hashlib.sha1(b'blob '+str(len(raw)).encode()+b'\0'+raw).hexdigest()
        if index.get(relative)==git_hash:
            row['git_blob']=git_hash
        elif '/fixtures/' in str(path):
            row['retained_fixture']=str(path)
        else:
            target=objects/digest
            if not target.exists():
                with target.open('xb') as stream: stream.write(raw)
            assert sha(target.read_bytes())==digest
            row['object']=str(target)
        rows.append(row)
    return rows
def tree_bytes(path):
    return sum(p.stat().st_size for p in path.rglob('*') if p.is_file() and not p.is_symlink())
def settings():
    result={name:subprocess.check_output(['rtk','proxy',*argv],cwd=ROOT,text=True).strip() for name,argv in [('rustc',['rustc','-Vv']),('cargo',['cargo','-V']),('cmake',['cmake','--version']),('go',['go','version']),('cxx',['c++','--version'])]}
    cache=Path('/build/CMakeCache.txt')
    if cache.exists(): result['cmake_cache_sha256']=sha(cache.read_bytes());result['rustaxa_on']='RUSTAXA_ENABLE:BOOL=ON' in cache.read_text()
    result['head']=subprocess.check_output(['rtk','proxy','git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
    return result
def selected_names_match(manifest):
    expected={row['path'] for row in json.loads(Path(manifest['source_manifest']['path']).read_bytes())}
    actual={str(path.relative_to(ROOT)) for path in source_paths(generated=manifest['declared_generated_inputs'])}
    return expected==actual

def test_binary_environment(parent, scratch, launch_path):
    # Cargo adjusts dynamic-library lookup for the selected binary. These are
    # public paths/package fields, not arbitrary ambient account credentials.
    environment=dict(parent['environment'])
    for name in ('LD_LIBRARY_PATH','DYLD_FALLBACK_LIBRARY_PATH','CARGO','CARGO_MANIFEST_DIR','CARGO_MANIFEST_PATH','CARGO_PKG_NAME','CARGO_PKG_VERSION','CARGO_PKG_VERSION_MAJOR','CARGO_PKG_VERSION_MINOR','CARGO_PKG_VERSION_PATCH','CARGO_PKG_VERSION_PRE'):
        if name in os.environ: environment[name]=os.environ[name]
    environment.update({'RUN_ARTIFACT_DIR':str(scratch),'TMPDIR':str(scratch/'legacy-temps'),'RUSTAXA_ERA_LAUNCH_MANIFEST':str(launch_path)})
    return environment

def verify_pinned_inputs(manifest):
    """Verify the parent's exact inventory without a repository tree walk."""
    source=manifest['source_manifest']
    assert sha(Path(source['path']).read_bytes())==source['sha256']
    for row in json.loads(Path(source['path']).read_bytes()):
        path=ROOT/row['path']
        assert path.is_file() and path.stat().st_size==row['bytes'] and sha(path.read_bytes())==row['sha256']
    assert settings()==manifest['toolchain_build']
    assert all(sha(Path(row['path']).read_bytes())==row['sha256'] for row in manifest['extra_inputs'])

def test_binary_runner(argv):
    """Run one Cargo-selected binary unchanged, then close only its own scratch."""
    assert argv and RUN_CONFIG is not None
    parent_path=Path(os.environ['RUSTAXA_ERA_LAUNCH_MANIFEST'])
    parent_raw=parent_path.read_bytes();parent=json.loads(parent_raw)
    assert parent.get('run_config')==RUN_CONFIG and parent.get('cargo_test_runner') is True
    assert parent['environment']['RUSTAXA_ERA_LAUNCH_MANIFEST']==str(parent_path)
    assert parent_path.is_relative_to(RUN/'transition-commands')
    verify_pinned_inputs(parent)
    executable=Path(argv[0]).resolve(strict=True)
    assert executable.is_file() and os.access(executable,os.X_OK)
    binary={'path':str(executable),'bytes':executable.stat().st_size,'sha256':sha(executable.read_bytes())}
    directory=parent_path.parent/'rust-test-binaries'/(executable.name+'-'+str(os.getpid())+'-'+str(time.time_ns()))
    directory.mkdir(parents=True,exist_ok=False)
    scratch=directory/'scratch'
    save(directory/'scratch-parent-registration.json',{'root':str(scratch),'new':True,'parent_launch':{'path':str(parent_path),'sha256':sha(parent_raw)},'executable':binary,'argv':argv})
    scratch.mkdir();legacy=scratch/'legacy-temps';legacy.mkdir()
    launch_path=directory/'launch.json'
    environment=test_binary_environment(parent,scratch,launch_path)
    stat=scratch.stat()
    manifest={'schema':1,'argv':argv,'cwd':os.getcwd(),'environment':environment,'source_manifest':parent['source_manifest'],'extra_inputs':parent['extra_inputs'],'toolchain_build':parent['toolchain_build'],'declared_generated_inputs':parent['declared_generated_inputs'],'run_config':RUN_CONFIG,'parent_launch':{'path':str(parent_path),'sha256':sha(parent_raw)},'executable':binary,'scratch_parent':str(scratch),'scratch_parent_identity':[stat.st_dev,stat.st_ino],'scratch_ledger':str(directory/'scratch-events.jsonl'),'unit_scratch_ledger':str(directory/'unit-scratch.jsonl'),'resources':parent['resources'],'completion_boundary':'exact Cargo-selected Rust test binary; all arguments preserved'}
    manifest['identity']=sha(json.dumps(manifest,sort_keys=True).encode())
    save(launch_path,manifest)
    print(json.dumps({'rust_test_binary_start':str(launch_path),'argv':argv,'executable':binary}),flush=True)
    started=time.monotonic()
    result=subprocess.run(argv,cwd=manifest['cwd'],env=environment)
    stable=parent_path.read_bytes()==parent_raw and sha(executable.read_bytes())==binary['sha256']
    try:
        verify_pinned_inputs(parent)
        stable=stable and selected_names_match(parent)
    except (AssertionError,OSError): stable=False
    outcome={'source_stable':stable,'source_manifest':manifest['source_manifest'],'argv':argv,'child_exit':result.returncode,'elapsed_seconds':time.monotonic()-started,'executable':binary,'parent_launch':manifest['parent_launch'],'build_relation':'Cargo selected this exact executable through its host runner after normal dependency checks on the pinned parent source; no source writer; parent later re-enumerates the complete source surface'}
    save(directory/'source-result.json',outcome)
    if stable and result.returncode==0: finish_unit_scratch(manifest)
    print(json.dumps({'rust_test_binary_result':str(directory/'source-result.json'),'child_exit':result.returncode,'source_stable':stable}),flush=True)
    return result.returncode if stable else 125

def finish_unit_scratch(manifest):
    ledger=Path(manifest['unit_scratch_ledger'])
    if not ledger.exists(): return
    command_result=json.loads((ledger.parent/'source-result.json').read_bytes())
    assert command_result['child_exit']==0 and command_result['source_stable'] is True
    assert command_result['source_manifest']==manifest['source_manifest'] and command_result['argv']==manifest['argv']
    rows=[json.loads(line) for line in ledger.read_text().splitlines()]
    registered={row['root']:row for row in rows if row['state']=='registered_before_creation'}
    closed={row['root']:row for row in rows if row['state']=='handles_closed'}
    created={row['root']:row for row in rows if row['state']=='created'}
    parent=Path(manifest['scratch_parent']); stat=parent.stat()
    assert [stat.st_dev,stat.st_ino]==manifest['scratch_parent_identity']
    for name, registration in registered.items():
        completion=closed.get(name)
        creation=created.get(name)
        history=[row for row in rows if row['root']==name]
        if creation is None or any(row.get('panicking',False) or row.get('tainted',False) for row in history): continue
        terminal=[row for row in history if row['state'] in ('handles_closed','disposal_requested')]
        if len(terminal)!=1: continue
        completion=terminal[0]
        if any(row.get('dependencies',0)!=0 for row in history): continue
        root=Path(name)
        assert root.is_relative_to(parent) and root!=parent and '..' not in root.parts
        assert root.is_relative_to(parent/'legacy-temps') or registration.get('completion_scope')=='whole_source_stable_command'
        if not root.exists(): continue
        assert all(not part.is_symlink() for part in root.parents) and not root.is_symlink()
        files=sorted(root.rglob('*'))
        assert all(not file.is_symlink() for file in files)
        stat=root.stat();assert [stat.st_dev,stat.st_ino]==completion['identity']==creation['identity']
        assert registration['launch']==creation['launch']==completion['launch']==manifest['identity']
        assert all(row.get('source_manifest',manifest['source_manifest'])==manifest['source_manifest'] for row in history)
        inventory=[{'path':str(file.relative_to(root)),'bytes':file.stat().st_size,'sha256':sha(file.read_bytes())} for file in files if file.is_file()]
        witness={'schema':1,'case':registration['test'],'registration':registration,'creation':creation,'completion':completion,'source_manifest':manifest['source_manifest'],'command':manifest['argv'],'child_exit':0,'source_stable':True,'assertions':'entire test command returned success after every assertion; child has exited and all handles are closed','actual_physical_files':inventory,'disposition':'dispose NEW registered unit scratch'}
        target=ledger.parent/('unit-case-'+sha(name.encode())+'.json');save(target,witness)
        assert json.loads(target.read_bytes())==witness
        shutil.rmtree(root)
        with ledger.open('a') as output:
            output.write(json.dumps({'state':'disposed_after_successful_source_stable_command','root':name,'witness':str(target),'before_bytes':sum(row['bytes'] for row in inventory),'after_bytes':0})+'\n');output.flush();os.fsync(output.fileno())
def output_limit(value):
    try: limit=int(value)
    except (TypeError,ValueError): raise argparse.ArgumentTypeError('output bound must be an integer')
    if not 1<=limit<=67108864: raise argparse.ArgumentTypeError('output bound must be 1..67108864 bytes')
    return limit

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--run-config',type=Path)
    parser.add_argument('--test-runner',action='store_true')
    parser.add_argument('--cargo-test-runner',action='store_true')
    parser.add_argument('--execute',type=Path)
    parser.add_argument('--name')
    parser.add_argument('--timeout',type=float,default=600)
    parser.add_argument('--max-output-bytes',type=output_limit,default=67108864)
    parser.add_argument('--retain',action='store_true')
    parser.add_argument('--input',type=Path,action='append',default=[])
    parser.add_argument('command',nargs=argparse.REMAINDER)
    args=parser.parse_args()
    selected=args.run_config or os.environ.get('RUSTAXA_ERA_RUN_CONFIG')
    if selected: configure_run(selected)
    if args.test_runner:
        return test_binary_runner(args.command)
    if args.execute:
        manifest=json.loads(args.execute.read_bytes())
        assert manifest.get('run_config')==RUN_CONFIG
        assert manifest['source_manifest']['sha256']==sha(Path(manifest['source_manifest']['path']).read_bytes())
        before=json.loads(Path(manifest['source_manifest']['path']).read_bytes())
        inputs=[ROOT/r['path'] for r in before if (ROOT/r['path']).parent==CORPUS]
        assert before==snapshot(inputs,manifest['declared_generated_inputs'])
        assert settings()==manifest['toolchain_build']
        assert all(sha(Path(row['path']).read_bytes())==row['sha256'] for row in manifest['extra_inputs'])
        started=time.monotonic()
        result=subprocess.run(manifest['argv'],cwd=ROOT,env=manifest['environment'])
        after=snapshot(inputs,manifest['declared_generated_inputs'])
        stable=before==after and selected_names_match(manifest) and settings()==manifest['toolchain_build'] and all(sha(Path(row['path']).read_bytes())==row['sha256'] for row in manifest['extra_inputs'])
        save(args.execute.parent/'source-result.json',{'source_stable':stable,'source_manifest':manifest['source_manifest'],'argv':manifest['argv'],'child_exit':result.returncode,'elapsed_seconds':time.monotonic()-started,'build_relation':'normal Cargo/CMake dependency checks on pinned source; no covered source writer during execution'})
        if stable and result.returncode==0: finish_unit_scratch(manifest)
        return result.returncode if stable else 125
    assert args.name and all(c.isalnum() or c in '-_' for c in args.name)
    command=args.command[1:] if args.command[:1]==['--'] else args.command
    assert command and 0<args.timeout<=1800
    directory=RUN/'transition-commands'/args.name; directory.mkdir(parents=True,exist_ok=False)
    scratch=RUN/'transition-scratch'/args.name
    assert not scratch.exists()
    # The command parent is recorded before its exclusive creation. Individual
    # test cases register their own roots before creation in the external ledger.
    save(directory/'scratch-parent-registration.json',{'root':str(scratch),'new':True,'case':'command parent','preauthorization_inventory':(json.loads(Path(RUN_CONFIG['path']).read_bytes())['baseline'] if RUN_CONFIG is not None else str(RUN/'transition-admission-01/pre-authorization-inventory.json'))})
    scratch.mkdir(parents=True,exist_ok=False)
    legacy=scratch/'legacy-temps'
    save(directory/'legacy-parent-registration.json',{'root':str(legacy),'new':True,'case':'legacy test temporary parent','disposition':'individual test success cleanup; failures retained by changed integration consumers'})
    legacy.mkdir()
    generated=[]
    if any('mainnet_era_reference.py' in arg or 'mainnet_era_estimate_reference.py' in arg for arg in command) and ('--record' in command or any('mainnet_era_estimate_reference.py' in arg for arg in command)):
        scenario=command[command.index('--scenario')+1]
        assert scenario in ('mainnet_106','cacti')
        generated=[str(CORPUS/(scenario+'.manifest.json'))]
    source=snapshot(generated=generated); source_path=directory/'source.json'; save(source_path,source)
    env={'RUN_ARTIFACT_DIR':str(scratch),'TMPDIR':str(legacy),'RUSTAXA_ERA_TASK_RUN':str(RUN),'RUSTAXA_ERA_RETAIN':'qualification' if args.retain else 'dispose','RUST_TEST_THREADS':'1','CMAKE_BUILD_PARALLEL_LEVEL':'12'}
    if RUN_CONFIG is not None: env['RUSTAXA_ERA_RUN_CONFIG']=RUN_CONFIG['path']
    env['GTEST_OUTPUT']='xml:'+str(directory/'gtest-results')+'/'
    if any('mainnet_era_' in arg and arg.endswith('.py') for arg in command):
        env['RUN_ARTIFACT_DIR']=str(RUN)
        env['RUN_SCHEDULER_MANIFEST']=str(RUN/'cpp-scheduler-05/manifest.json')
    for name in ('CARGO_TARGET_DIR','RUSTFLAGS','CARGO_BUILD_JOBS','RUSTC_WRAPPER','PATH','HOME','RUSTUP_TOOLCHAIN','RUSTUP_HOME','CARGO_HOME','USER','LOGNAME','LANG','LC_ALL','LD_LIBRARY_PATH','PKG_CONFIG_PATH','CMAKE_PREFIX_PATH','CC','CXX','AR','GOPATH','GOMODCACHE','GOFLAGS','GOCACHE','CGO_ENABLED','SCCACHE_DIR'):
        if name in os.environ: env[name]=os.environ[name]
    assert not any(os.environ.get(name) for name in ('MAINNET_ERA_API_PAIR','CACTI_ERA_API_LOCAL_PAIR','CACTI_ERA_API_PUBLIC_PAIR'))
    manifest_path=directory/'launch.json'; env['RUSTAXA_ERA_LAUNCH_MANIFEST']=str(manifest_path)
    build_settings=settings()
    if args.cargo_test_runner:
        assert RUN_CONFIG is not None
        host=next(line.split(': ',1)[1] for line in build_settings['rustc'].splitlines() if line.startswith('host: '))
        runner_key='CARGO_TARGET_'+host.upper().replace('-','_')+'_RUNNER'
        env[runner_key]='rtk proxy python3 '+str(Path(__file__).resolve())+' --test-runner'
    extras=set(args.input)
    if RUN_CONFIG is not None:
        config=json.loads(Path(RUN_CONFIG['path']).read_bytes()); extras.update(Path(config[key]['path']) for key in ('authority','baseline')); extras.add(Path(RUN_CONFIG['path']))
    for path in args.input:
        value=json.loads(path.read_bytes())
        # Explicit effective CMake overlay inputs are outside the checkout.
        # Pin every generated byte used by the compiler, not only its recipe.
        for effective in value.get('effective_files', []): extras.add(Path(effective))
        for shard_set in value.get('shards',{}).values():
            for group in shard_set.values():
                for row in group:
                    command_path=Path(row['stream']['capture'])/'command.json'; extras.add(command_path)
                    for source in json.loads(command_path.read_bytes())['effective_sources'].values(): extras.add(Path(source.get('object',str(OLD_RUN/'source-objects'/source['sha256']))))
    extra_pins=[{'path':str(path.resolve()),'sha256':sha(path.read_bytes()),'bytes':path.stat().st_size} for path in sorted(extras)]
    source_pin={'path':str(source_path),'sha256':sha(source_path.read_bytes())}
    resource=resources()
    baseline=resource['baseline_actual_bytes']
    current=resource['prior_charged_bytes']+max(0,tree_bytes(RUN)+tree_bytes(CORPUS)-baseline)+2*1024*1024
    oracle=any('mainnet_era_' in arg and arg.endswith('.py') for arg in command)
    reserve=(128 if oracle else 96 if args.retain else 64)*1024*1024
    assert current+reserve+args.max_output_bytes+16*1024*1024<=resource['retained_live_cap_bytes'], ('peak admission',current,reserve)
    stat=scratch.stat()
    manifest={'schema':1,'argv':command,'environment':env,'source_manifest':source_pin,'extra_inputs':extra_pins,'toolchain_build':build_settings,'peak_admission':{'charged_live_bytes':current,'case_peak_reserve':reserve,'failure_reserve':args.max_output_bytes,'output_reserve_bytes':args.max_output_bytes,'closeout_reserve':16*1024*1024},'declared_generated_inputs':generated,'scratch_parent':str(scratch),'scratch_parent_identity':[stat.st_dev,stat.st_ino],'scratch_ledger':str(directory/'scratch-events.jsonl'),'unit_scratch_ledger':str(directory/'unit-scratch.jsonl'),'resources':resource}
    manifest['max_output_bytes']=args.max_output_bytes
    if RUN_CONFIG is not None: manifest['run_config']=RUN_CONFIG
    if args.cargo_test_runner: manifest['cargo_test_runner']=True
    manifest['identity']=sha(json.dumps(manifest,sort_keys=True).encode())
    save(manifest_path,manifest)
    inner=['rtk','proxy','python3',str(Path(__file__).resolve())]
    if RUN_CONFIG is not None: inner += ['--run-config',RUN_CONFIG['path']]
    inner += ['--execute',str(manifest_path)]
    code,status=capture(directory/'capture',inner,timeout_seconds=args.timeout,max_output_bytes=args.max_output_bytes)
    print(json.dumps({'directory':str(directory),'child_exit':status['child_exit'],'capture_complete':status['capture_complete'],'runner_exit':code}))
    return code
if __name__=='__main__': sys.exit(main())
