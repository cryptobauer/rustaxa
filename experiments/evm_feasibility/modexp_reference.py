#!/usr/bin/env python3
"""Reproduce bounded pinned Go MODEXP fixtures; --record updates them."""
import argparse,hashlib,json,os,pathlib,subprocess,tempfile
from reference import REVISIONS,ROOT
HERE=pathlib.Path(__file__).resolve().parent
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--record',action='store_true');a=p.parse_args();src=pathlib.Path(os.environ.get('TARAXA_EVM_SOURCE',ROOT/'submodules/taraxa-evm'));exporter=(HERE/'modexp_reference.go').read_bytes();arts={}
 for label,rev in REVISIONS.items():
  with tempfile.TemporaryDirectory(prefix='rustaxa-modexp-') as d:
   tree=pathlib.Path(d);archive=subprocess.check_output(['git','-C',str(src),'archive',rev]);subprocess.run(['tar','-x','-C',d],input=archive,check=True);cmd=tree/'cmd/modexp_reference';cmd.mkdir(parents=True);(cmd/'main.go').write_bytes(exporter);arts[label]=subprocess.check_output(['go','run','-mod=readonly','./cmd/modexp_reference'],cwd=tree)
 if arts['public']!=arts['local']:raise RuntimeError('pinned MODEXP outputs disagree')
 f=HERE/'fixtures';m={'schema':1,'references':REVISIONS,'go_version':subprocess.check_output(['go','version'],text=True).strip(),'scope':'Direct Californicum address5 RequiredGas/Run bounded MODEXP primitives; no EVM admission or routing claim','sha256':{k:hashlib.sha256(v).hexdigest() for k,v in arts.items()},'exporter_sha256':hashlib.sha256(exporter).hexdigest()}
 if a.record:
  for k,v in arts.items():(f/f'modexp_{k}.json').write_bytes(v)
  (f/'modexp_manifest.json').write_text(json.dumps(m,indent=2)+'\n')
 else:
  if json.loads((f/'modexp_manifest.json').read_text())!=m:raise RuntimeError('MODEXP manifest mismatch')
  for k,v in arts.items():
   if (f/f'modexp_{k}.json').read_bytes()!=v:raise RuntimeError(f'MODEXP fixture mismatch: {k}')
 print('Both pinned MODEXP references executed and '+('recorded' if a.record else 'reproduced'))
if __name__=='__main__':main()
