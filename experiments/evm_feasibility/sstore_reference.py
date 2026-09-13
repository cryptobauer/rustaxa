#!/usr/bin/env python3
"""Reproduce the additive pinned Go SSTORE corpus; --record updates its files."""

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = pathlib.Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    source = pathlib.Path(os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm"))
    exporter = (HERE / "sstore_reference.go").read_bytes()
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-sstore-") as directory:
            tree = pathlib.Path(directory)
            archive = subprocess.check_output(["git", "-C", str(source), "archive", revision])
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd/sstore_reference"
            command.mkdir(parents=True)
            (command / "sstore_reference.go").write_bytes(exporter)
            artifacts[label] = subprocess.check_output(
                ["go", "run", "-mod=readonly", "./cmd/sstore_reference"], cwd=tree
            )
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("pinned Go SSTORE outputs disagree")
    fixtures = HERE / "fixtures"
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"], text=True).strip(),
        "scope": "Synthetic direct Go EVM/state_evm SSTORE sequences; no full E3/frame/routing claim",
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "exporter_sha256": hashlib.sha256(exporter).hexdigest(),
    }
    manifest_path = fixtures / "sstore_manifest.json"
    if args.record:
        for label, data in artifacts.items():
            (fixtures / f"sstore_{label}.json").write_bytes(data)
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        if json.loads(manifest_path.read_text()) != manifest:
            raise RuntimeError("SSTORE manifest mismatch")
        for label, data in artifacts.items():
            if (fixtures / f"sstore_{label}.json").read_bytes() != data:
                raise RuntimeError(f"SSTORE fixture mismatch: {label}")
    print("Both pinned SSTORE references agree; " + ("recorded" if args.record else "reproduced"))


if __name__ == "__main__":
    main()
