#!/usr/bin/env python3
"""Run the additive pinned Go SSTORE oracle in both archived reference trees."""
import hashlib, json, os, pathlib, subprocess, tempfile
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SOURCE = pathlib.Path(os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm"))
REVISIONS = {"public": "6c7e5338b22d5e596cc2365a88d1f94840e1ee1b", "local": "bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418"}
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
for label, data in artifacts.items(): (fixtures / f"sstore_{label}.json").write_bytes(data)
manifest = {"schema": 1, "references": REVISIONS, "go_version": subprocess.check_output(["go", "version"]).decode().strip(), "scope": "Synthetic direct Go EVM/state_evm SSTORE sequences; no full E3/frame/routing claim", "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()}, "exporter_sha256": hashlib.sha256((HERE / "sstore_reference.go").read_bytes()).hexdigest()}
(fixtures / "sstore_manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print("Both pinned SSTORE references executed and agreed")
