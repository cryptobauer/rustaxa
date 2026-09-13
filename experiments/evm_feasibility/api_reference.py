#!/usr/bin/env python3
"""Reproduce pinned public DryRunner fixtures; use --record to update them.

The exporter archives each known source revision into a disposable directory,
copies only api_reference.go into it, and invokes the real public
state_dry_runner.DryRunner.Apply entrypoint. It never edits the submodule.
"""

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile

from reference import REVISIONS

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = pathlib.Path(
    os.environ.get("TARAXA_EVM_SOURCE", "/workspaces/rustaxa-evm/submodules/taraxa-evm")
)
GO_SOURCE = HERE / "api_reference.go"


def run_reference(revision: str) -> bytes:
    with tempfile.TemporaryDirectory(prefix="rustaxa-api-reference-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        command = tree / "cmd/api_reference"
        command.mkdir(parents=True)
        (command / GO_SOURCE.name).write_bytes(GO_SOURCE.read_bytes())
        return subprocess.check_output(
            ["go", "run", "-mod=readonly", "./cmd/api_reference"], cwd=tree
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    parsed = {label: json.loads(data) for label, data in artifacts.items()}
    differences = {
        "byte_identical": artifacts["public"] == artifacts["local"],
        "semantic_note": (
            "The bounded DryRunner source and outputs are identical at the public/local pins. "
            "The local pin additionally contains concrete-state lifecycle work outside this public API fixture."
        ),
    }
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"]).decode().strip(),
        "scope": (
            "Synthetic ordinary CALL/CREATE through state_dry_runner.DryRunner.Apply over a complete "
            "in-memory committed trie; no native dispatch, RPC defaults/formatting, estimates, traces, "
            "RocksDB, network replay, persistence, or production routing claim"
        ),
        "reference_config": parsed["public"]["reference_config"],
        "fixture_schema": {
            "state_before/state_after": "committed descriptor plus account/code/slot reads and encoded seed rows",
            "cases[].input": "caller-supplied transaction fields before DryRunner nonce replacement",
            "cases[].output": "effective nonce and exact vm.ExecutionResult fields returned by DryRunner",
            "repeated": "two independent applications of the same call against the same committed state",
        },
        "public_local": differences,
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "exporter_sha256": hashlib.sha256(GO_SOURCE.read_bytes()).hexdigest(),
    }
    fixtures = HERE / "fixtures"
    if args.record:
        fixtures.mkdir(exist_ok=True)
        for label, data in artifacts.items():
            (fixtures / f"api_{label}.json").write_bytes(data)
        (fixtures / "api_manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        saved = json.loads((fixtures / "api_manifest.json").read_text())
        for field in (
            "schema",
            "references",
            "scope",
            "reference_config",
            "fixture_schema",
            "public_local",
            "sha256",
            "exporter_sha256",
        ):
            assert saved[field] == manifest[field], f"API reference manifest mismatch: {field}"
        for label, data in artifacts.items():
            assert data == (fixtures / f"api_{label}.json").read_bytes(), f"API fixture mismatch: {label}"
    print("Both pinned DryRunner references executed and " + ("recorded" if args.record else "reproduced"))
    print("Public/local identical:", differences["byte_identical"])


if __name__ == "__main__":
    main()
