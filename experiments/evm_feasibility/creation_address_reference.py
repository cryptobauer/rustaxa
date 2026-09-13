#!/usr/bin/env python3
"""Reproduce pinned Go CreateAddress fixtures; use --record to update them."""
import argparse, hashlib, json, os, pathlib, subprocess, tempfile
from reference import REVISIONS, ROOT

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = pathlib.Path(os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm"))
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--record", action="store_true")
args = parser.parse_args()
artifacts = {}
for label, revision in REVISIONS.items():
    with tempfile.TemporaryDirectory(prefix="rustaxa-create-address-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        command = tree / "cmd/creation_address_reference"; command.mkdir(parents=True)
        (command / "creation_address_reference.go").write_bytes((HERE / "creation_address_reference.go").read_bytes())
        artifacts[label] = subprocess.check_output(["go", "run", "-mod=readonly", "./cmd/creation_address_reference"], cwd=tree)
assert artifacts["public"] == artifacts["local"], "pinned CreateAddress outputs disagree"
manifest = {"schema": 1, "references": REVISIONS, "go_version": subprocess.check_output(["go", "version"]).decode().strip(), "scope": "Synthetic crypto.CreateAddress nonce-boundary vectors; no frame, state, DB, or routing claim", "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()}, "exporter_sha256": hashlib.sha256((HERE / "creation_address_reference.go").read_bytes()).hexdigest()}
fixtures = HERE / "fixtures"
if args.record:
    for label, data in artifacts.items(): (fixtures / f"creation_address_{label}.json").write_bytes(data)
    (fixtures / "creation_address_manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
else:
    saved = json.loads((fixtures / "creation_address_manifest.json").read_text())
    assert saved["references"] == REVISIONS and saved["sha256"] == manifest["sha256"] and saved["exporter_sha256"] == manifest["exporter_sha256"]
    for label, data in artifacts.items(): assert (fixtures / f"creation_address_{label}.json").read_bytes() == data
print("Both pinned CreateAddress references executed and " + ("recorded" if args.record else "reproduced"))
