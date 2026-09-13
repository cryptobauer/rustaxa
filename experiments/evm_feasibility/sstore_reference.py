#!/usr/bin/env python3
"""Run the additive pinned Go SSTORE oracle in both archived reference trees."""
import argparse, hashlib, json, os, pathlib, subprocess, tempfile
from reference import REVISIONS, ROOT
HERE = pathlib.Path(__file__).resolve().parent
SOURCE = pathlib.Path(os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm"))
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--record", action="store_true")
args = parser.parse_args()
artifacts = {}
for label, revision in REVISIONS.items():
    with tempfile.TemporaryDirectory(prefix="rustaxa-sstore-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        command = tree / "cmd/sstore_reference"; command.mkdir(parents=True)
        (command / "sstore_reference.go").write_bytes((HERE / "sstore_reference.go").read_bytes())
        artifacts[label] = subprocess.check_output(["go", "run", "-mod=readonly", "./cmd/sstore_reference"], cwd=tree)
assert artifacts["public"] == artifacts["local"], "pinned Go SSTORE outputs disagree"
fixtures = HERE / "fixtures"
manifest = {"schema": 1, "references": REVISIONS, "go_version": subprocess.check_output(["go", "version"]).decode().strip(), "scope": "Synthetic direct Go EVM/state_evm SSTORE sequences; no full E3/frame/routing claim", "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()}, "exporter_sha256": hashlib.sha256((HERE / "sstore_reference.go").read_bytes()).hexdigest()}
if args.record:
    for label, data in artifacts.items(): (fixtures / f"sstore_{label}.json").write_bytes(data)
    (fixtures / "sstore_manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
else:
    saved = json.loads((fixtures / "sstore_manifest.json").read_text())
    assert saved["references"] == REVISIONS and saved["sha256"] == manifest["sha256"]
    assert saved["exporter_sha256"] == manifest["exporter_sha256"]
print("Both pinned SSTORE references executed and " + ("recorded" if args.record else "reproduced"))
