#!/usr/bin/env python3
"""Reproduce the dual-pin Taraxa Ficus/Cacti MCOPY corpus."""

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = pathlib.Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures" / "mcopy"


def fail(message):
    raise RuntimeError(message)


def checked_output(command, **kwargs):
    try:
        return subprocess.check_output(command, **kwargs)
    except subprocess.CalledProcessError as error:
        fail(f"command failed ({error.returncode}): {' '.join(map(str, command))}")


def validate_artifact(label, data):
    try:
        document = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"{label} emitted invalid JSON: {error}")
    rows = document.get("mcopy")
    if not isinstance(rows, list) or not rows:
        fail(f"{label} omitted MCOPY rows")
    names = set()
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            fail(f"{label} row {index} is not an object")
        required = {
            "name",
            "phase",
            "code",
            "gas_used",
            "output",
            "execution_error",
            "outer_error",
        }
        if set(row) != required:
            fail(f"{label} row {index} has unexpected fields")
        if row["name"] in names:
            fail(f"{label} duplicate row name: {row['name']}")
        names.add(row["name"])
        if row["phase"] not in ("californicum", "ficus", "cacti"):
            fail(f"{label} row {index} has invalid phase")
        for field in ("code", "output"):
            try:
                bytes.fromhex(row[field])
            except (TypeError, ValueError) as error:
                fail(f"{label} row {index} invalid {field}: {error}")
        if not isinstance(row["gas_used"], int) or not 0 <= row["gas_used"] <= 100000:
            fail(f"{label} row {index} has invalid gas")
        if not isinstance(row["execution_error"], str) or not isinstance(
            row["outer_error"], str
        ):
            fail(f"{label} row {index} has invalid error")
    needed = {
        "before-ficus",
        "ficus-zero-length-wide-dst",
        "cacti-inherits-zero-length-wide-dst",
        "ficus-copy-forward-overlap",
        "ficus-copy-backward-overlap",
        "cacti-copy-forward-overlap",
        "ficus-expand-empty-memory",
        "ficus-copy-two-words",
        "ficus-expansion-out-of-gas",
        "ficus-stack-underflow",
    }
    missing = needed - names
    if missing:
        fail(f"{label} omitted required cases: {sorted(missing)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    source = pathlib.Path(os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm"))
    exporter = (HERE / "mcopy_reference.go").read_bytes()
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-mcopy-") as directory:
            tree = pathlib.Path(directory)
            archive = checked_output(["git", "-C", str(source), "archive", revision])
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd/mcopy_reference"
            command.mkdir(parents=True)
            (command / "mcopy_reference.go").write_bytes(exporter)
            data = checked_output(
                ["go", "run", "-mod=readonly", "./cmd/mcopy_reference"], cwd=tree
            )
            validate_artifact(label, data)
            artifacts[label] = data
    if artifacts["public"] != artifacts["local"]:
        fail("pinned Go MCOPY outputs disagree")

    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": checked_output(["go", "version"], text=True).strip(),
        "scope": (
            "EVM-level Californicum/Ficus/Cacti MCOPY activation, overlap, memory "
            "expansion, zero-length wide offset, stack and gas boundaries; no state, "
            "native registry, publication or production-routing claim"
        ),
        "transaction_gas_limit": 100000,
        "transaction_intrinsic_gas": 21000,
        "sha256": {
            label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()
        },
        "exporter_sha256": hashlib.sha256(exporter).hexdigest(),
    }
    expected = {
        "public.json": artifacts["public"],
        "local.json": artifacts["local"],
        "manifest.json": (json.dumps(manifest, indent=2) + "\n").encode(),
    }
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for name, data in expected.items():
            (FIXTURES / name).write_bytes(data)
        return
    for name, data in expected.items():
        path = FIXTURES / name
        if not path.is_file():
            fail(f"missing fixture: {path}; run with --record after review")
        if path.read_bytes() != data:
            fail(f"fixture differs: {path}; run with --record only for an intentional update")


if __name__ == "__main__":
    main()
