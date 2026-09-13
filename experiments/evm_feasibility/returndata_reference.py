#!/usr/bin/env python3
"""Reproduce RETURNDATACOPY execution from two immutable Go source archives."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile
from reference import ROOT, REVISIONS

HERE = pathlib.Path(__file__).resolve().parent

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    sources = {name: (HERE / name).read_bytes() for name in ["mcopy_reference.go", "returndata_reference.go"]}
    if sources["mcopy_reference.go"].count(b"func main() {") != 1:
        raise RuntimeError("MCOPY helper main extraction is ambiguous")
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-returndata-reference-") as directory:
            tree = pathlib.Path(directory)
            archive = subprocess.check_output(["git", "-C", str(ROOT / "submodules/taraxa-evm"), "archive", revision])
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd/returndata_reference"
            command.mkdir(parents=True)
            for name, data in sources.items():
                if name == "mcopy_reference.go":
                    data = data.replace(b"func main() {", b"func mcopyReferenceMain() {")
                (command / name).write_bytes(data)
            artifacts[label] = subprocess.check_output(["go", "run", "-mod=readonly", "./cmd/returndata_reference"], cwd=tree)
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("Pinned RETURNDATACOPY outputs differ")
    manifest = {"schema": 1, "references": REVISIONS,
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "source_sha256": {name: hashlib.sha256(data).hexdigest() for name, data in sources.items()},
        "scope": "actual EVM.Main, 23 RETURNDATACOPY programs in each of three phases, including source bounds, gas/memory overflow and observed reference panics"}
    artifacts["manifest"] = (json.dumps(manifest, indent=2) + "\n").encode()
    for label, data in artifacts.items():
        path = HERE / "fixtures" / ("returndata_" + label + ".json")
        if args.record:
            path.write_bytes(data)
        elif path.read_bytes() != data:
            raise RuntimeError("RETURNDATACOPY artifact mismatch: " + label)
    print("Both pinned RETURNDATACOPY fixtures " + ("recorded" if args.record else "reproduced"))

if __name__ == "__main__":
    main()
