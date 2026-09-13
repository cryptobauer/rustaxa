#!/usr/bin/env python3
"""Reproduce exact byte-valued revert reasons from both immutable Go ABI pins."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile
from reference import REVISIONS, ROOT

HERE = pathlib.Path(__file__).resolve().parent

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    exporter = (HERE / "revert_reference.go").read_bytes()
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-revert-") as directory:
            archive = subprocess.check_output(["git", "-C", str(ROOT / "submodules/taraxa-evm"), "archive", revision])
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            tree = pathlib.Path(directory)
            command = tree / "cmd/revert_reference"
            command.mkdir(parents=True)
            (command / "main.go").write_bytes(exporter)
            data = subprocess.check_output(["go", "run", "-mod=readonly", "./cmd/revert_reference"], cwd=tree)
            rows = json.loads(data)
            if len(rows) != 17 or len({row["name"] for row in rows}) != 17:
                raise RuntimeError("incomplete revert corpus")
            artifacts[label] = data
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("pinned revert decoders diverge")
    manifest = {"schema": 1, "references": REVISIONS,
                "scope": "actual abi.UnpackRevert bytes; DryRunner suffix composition; no RPC JSON string normalization",
                "exporter_sha256": hashlib.sha256(exporter).hexdigest(),
                "sha256": {key: hashlib.sha256(value).hexdigest() for key, value in artifacts.items()}}
    expected = {label + ".json": data for label, data in artifacts.items()}
    expected["manifest.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    target = HERE / "fixtures/revert"
    if args.record:
        target.mkdir(parents=True, exist_ok=True)
        for name, data in expected.items():
            (target / name).write_bytes(data)
    else:
        for name, data in expected.items():
            if (target / name).read_bytes() != data:
                raise RuntimeError("revert fixture differs: " + name)

if __name__ == "__main__":
    main()
