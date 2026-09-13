#!/usr/bin/env python3
"""Export actual raw logger refund facts from immutable disposable Go archives."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile
from reference import REVISIONS, ROOT

HERE = pathlib.Path(__file__).resolve().parent
TRACE_PATH = 'taraxa/state/state_dry_runner/trace_runner.go'
TRACE_SHA = '12fcddf4955e963d91318e5d18725868ecb29f5c32ae32db0fa73b7d99f65efd'

def replace_once(data, old, new):
    if data.count(old) != 1:
        raise RuntimeError('ambiguous observer/seed insertion: ' + old.decode())
    return data.replace(old,new)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record',action='store_true')
    args = parser.parse_args()
    exporter = (HERE / 'trace_refund_reference.go').read_bytes()
    api = (HERE / 'api_reference.go').read_bytes()
    start = api.index(b'func apiSeed() *apiMemory {')
    end = api.index(b'\ntype apiAccountObservation',start)
    seed = api[start:end]
    seed = replace_once(seed,b'func apiSeed() *apiMemory {',b'func apiSeedWithCode(callCode []byte) *apiMemory {')
    seed = replace_once(seed,b'\tcallCode := apiCallCode()\n',b'')
    support = replace_once(api,b'func main() {',b'func apiReferenceMain() {') + b'\n' + seed
    artifacts = {}
    for label,revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix='rustaxa-trace-refund-') as directory:
            tree = pathlib.Path(directory)
            archive = subprocess.check_output(['git','-C',str(ROOT/'submodules/taraxa-evm'),'archive',revision])
            subprocess.run(['tar','-x','-C',directory],input=archive,check=True)
            trace = tree / TRACE_PATH
            original = trace.read_bytes()
            if hashlib.sha256(original).hexdigest() != TRACE_SHA:
                raise RuntimeError('unreviewed TraceRunner source')
            observed = replace_once(original,b'vm.FormatLogs(tracer.StructLogs())',b'tracer.StructLogs()')
            observed = replace_once(observed,b'[]vm.StructLogRes',b'[]vm.StructLog')
            trace.write_bytes(observed)
            command = tree/'cmd/trace_refund_reference'
            command.mkdir(parents=True)
            (command/'main.go').write_bytes(exporter)
            (command/'api_reference.go').write_bytes(support)
            artifacts[label] = subprocess.check_output(['go','run','-mod=readonly','./cmd/trace_refund_reference'],cwd=tree)
    if artifacts['public'] != artifacts['local']:
        raise RuntimeError('pinned raw logger facts diverge')
    manifest = {'schema':1,'references':REVISIONS,'observed_source':TRACE_PATH,'observed_source_sha256':TRACE_SHA,
                'api_support_sha256':hashlib.sha256(api).hexdigest(), 'exporter_sha256':hashlib.sha256(exporter).hexdigest(),
                'sha256':{label:hashlib.sha256(data).hexdigest() for label,data in artifacts.items()},
                'scope':'actual raw StructLogs via observer-only disposable export; four slot7 clear/restore/revert programs and three sequential refund/transient cases; no formatted trace API claim'}
    artifacts['manifest'] = (json.dumps(manifest,indent=2)+'\n').encode()
    target = HERE/'fixtures/trace_refund'
    if args.record:
        target.mkdir(parents=True,exist_ok=True)
    for label,data in artifacts.items():
        path = target/(label+'.json')
        if args.record:
            path.write_bytes(data)
        elif path.read_bytes()!=data:
            raise RuntimeError('raw logger fixture mismatch: '+label)
    print('Both pinned raw logger refund fixtures '+('recorded' if args.record else 'reproduced'))

if __name__ == '__main__':
    main()
