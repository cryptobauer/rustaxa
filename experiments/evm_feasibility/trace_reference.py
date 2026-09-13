#!/usr/bin/env python3
"""Reproduce bounded traces from two immutable Go source archives."""
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
    sources = {name: (HERE / name).read_bytes() for name in ["api_reference.go", "trace_reference.go"]}
    if sources["api_reference.go"].count(b"func main() {") != 1:
        raise RuntimeError("API helper main extraction is ambiguous")
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-trace-reference-") as directory:
            tree = pathlib.Path(directory)
            archive = subprocess.check_output(["git", "-C", str(ROOT / "submodules/taraxa-evm"), "archive", revision])
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd/trace_reference"
            command.mkdir(parents=True)
            for name, data in sources.items():
                if name == "api_reference.go":
                    data = data.replace(b"func main() {", b"func apiReferenceMain() {")
                (command / name).write_bytes(data)
            artifacts[label] = subprocess.check_output(["go", "run", "-mod=readonly", "./cmd/trace_reference"], cwd=tree)
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("Pinned trace outputs differ")
    manifest = {"schema": 1, "references": REVISIONS,
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "source_sha256": {name: hashlib.sha256(data).hexdigest() for name, data in sources.items()},
        "scope": "actual TraceRunner, seven ordinary scenarios and five tracer modes; no Rust parity claimed by this oracle alone"}
    artifacts["manifest"] = (json.dumps(manifest, indent=2) + "\n").encode()
    for label, data in artifacts.items():
        path = HERE / "fixtures" / ("trace_" + label + ".json")
        if args.record:
            path.write_bytes(data)
        elif path.read_bytes() != data:
            raise RuntimeError("Trace artifact mismatch: " + label)
    print("Both pinned TraceRunner fixtures " + ("recorded" if args.record else "reproduced"))

if __name__ == "__main__":
    main()
