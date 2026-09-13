#!/usr/bin/env python3
"""Run and verify the bounded two-period S4 oracle in both pinned Go trees."""

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = pathlib.Path(os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm"))
FIXTURES = HERE / "fixtures"


def run_reference(revision: str) -> bytes:
    with tempfile.TemporaryDirectory(prefix="rustaxa-s4-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        command = tree / "cmd/s4_reference"
        command.mkdir(parents=True)
        (command / "s4_reference.go").write_bytes((HERE / "s4_reference.go").read_bytes())
        return subprocess.check_output(
            ["go", "run", "-mod=readonly", "./cmd/s4_reference"], cwd=tree
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("pinned Go S4 outputs disagree")

    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"]).decode().strip(),
        "scope": (
            "Synthetic direct two-period Go EVM/TransitionState/TrieSink execution with "
            "signed chain-841 inputs, FinalChain receipt RLP, and exact PendingBlockState "
            "physical-key projection; no application headers, rewards, RocksDB open, or "
            "production-routing claim"
        ),
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "exporter_sha256": hashlib.sha256((HERE / "s4_reference.go").read_bytes()).hexdigest(),
    }
    if args.record:
        FIXTURES.mkdir(exist_ok=True)
        for label, data in artifacts.items():
            (FIXTURES / f"s4_{label}.json").write_bytes(data)
        (FIXTURES / "s4_manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        saved = json.loads((FIXTURES / "s4_manifest.json").read_text())
        if saved != manifest:
            raise RuntimeError("saved S4 metadata differs")
        for label, data in artifacts.items():
            if data != (FIXTURES / f"s4_{label}.json").read_bytes():
                raise RuntimeError(f"saved {label} fixture differs")
    print("Both pinned S4 references executed and " + ("recorded" if args.record else "reproduced"))


if __name__ == "__main__":
    main()
