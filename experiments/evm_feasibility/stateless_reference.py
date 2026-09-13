#!/usr/bin/env python3
"""Reproduce pinned direct Go precompile primitive fixtures; --record updates."""
import argparse, hashlib, json, os, pathlib, subprocess, tempfile
from reference import REVISIONS, ROOT
HERE=pathlib.Path(__file__).resolve().parent
SOURCE=pathlib.Path(os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm"))
p=argparse.ArgumentParser(description=__doc__);p.add_argument("--record",action="store_true");args=p.parse_args(); artifacts={}
for label,revision in REVISIONS.items():
 with tempfile.TemporaryDirectory(prefix="rustaxa-stateless-") as directory:
  tree=pathlib.Path(directory); archive=subprocess.check_output(["git","-C",str(SOURCE),"archive",revision]); subprocess.run(["tar","-x","-C",directory],input=archive,check=True)
  cmd=tree/"cmd/stateless_reference";cmd.mkdir(parents=True);(cmd/"main.go").write_bytes((HERE/"stateless_reference.go").read_bytes());artifacts[label]=subprocess.check_output(["go","run","-mod=readonly","./cmd/stateless_reference"],cwd=tree)
assert artifacts["public"]==artifacts["local"],"pinned stateless outputs disagree"
manifest={"schema":1,"references":REVISIONS,"go_version":subprocess.check_output(["go","version"],text=True).strip(),"scope":"Direct Californicum addresses 1..4 RequiredGas/Run primitives; no EVM admission or routing claim","exporter_sha256":hashlib.sha256((HERE/"stateless_reference.go").read_bytes()).hexdigest(),"sha256":{k:hashlib.sha256(v).hexdigest() for k,v in artifacts.items()}}
f=HERE/"fixtures"
if args.record:
 for k,v in artifacts.items():(f/f"stateless_{k}.json").write_bytes(v)
 (f/"stateless_manifest.json").write_text(json.dumps(manifest,indent=2)+"\n")
else:
 assert json.loads((f/"stateless_manifest.json").read_text())==manifest
 for k,v in artifacts.items():assert (f/f"stateless_{k}.json").read_bytes()==v
print("Both pinned stateless references executed and "+("recorded" if args.record else "reproduced"))
