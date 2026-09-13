#!/usr/bin/env python3
"""Reproduce the bounded pinned Go modexp corpus; --record updates its files."""

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
    exporter = (HERE / "modexp_reference.go").read_bytes()
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-modexp-") as directory:
            tree = pathlib.Path(directory)
            archive = subprocess.check_output(["git", "-C", str(source), "archive", revision])
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd/modexp_reference"
            command.mkdir(parents=True)
            (command / "modexp_reference.go").write_bytes(exporter)
            artifacts[label] = subprocess.check_output(
                ["go", "run", "-mod=readonly", "./cmd/modexp_reference"], cwd=tree
            )
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("pinned Go modexp outputs disagree")
    fixtures = HERE / "fixtures"
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"], text=True).strip(),
        "scope": "Direct Californicum address 5 RequiredGas/Run primitives; no EVM admission or routing claim",
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "exporter_sha256": hashlib.sha256(exporter).hexdigest(),
    }
    manifest_path = fixtures / "modexp_manifest.json"
    if args.record:
        for label, data in artifacts.items():
            (fixtures / f"modexp_{label}.json").write_bytes(data)
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        if json.loads(manifest_path.read_text()) != manifest:
            raise RuntimeError("modexp manifest mismatch")
        for label, data in artifacts.items():
            if (fixtures / f"modexp_{label}.json").read_bytes() != data:
                raise RuntimeError(f"modexp fixture mismatch: {label}")
    print("Both pinned modexp references agree; " + ("recorded" if args.record else "reproduced"))


if __name__ == "__main__":
    main()
