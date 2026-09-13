#!/usr/bin/env python3
"""Reproduce the bounded dual-pin Go Cacti P-256 corpus.

The default mode verifies checked-in bytes and hashes. ``--record`` replaces
the three owned fixture files only after both immutable revisions agree.
"""

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = pathlib.Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures" / "p256"


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
    rows = document.get("p256")
    if not isinstance(rows, list) or not rows:
        fail(f"{label} omitted P-256 rows")
    names = set()
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            fail(f"{label} row {index} is not an object")
        if set(row) != {"name", "input", "required_gas", "output", "error"}:
            fail(f"{label} row {index} has unexpected fields")
        name = row["name"]
        if not isinstance(name, str) or name in names:
            fail(f"{label} duplicate or invalid row name: {name!r}")
        names.add(name)
        for field in ("input", "output"):
            try:
                bytes.fromhex(row[field])
            except (TypeError, ValueError) as error:
                fail(f"{label} row {index} invalid {field}: {error}")
        if row["required_gas"] != 6900:
            fail(f"{label} row {index} has wrong gas")
        if row["error"] != "":
            fail(f"{label} row {index} unexpectedly errors")
        if row["output"] not in ("", "00" * 31 + "01"):
            fail(f"{label} row {index} has non-boolean output")
    needed = {
        "empty",
        "zero-159",
        "zero-160",
        "zero-161",
        "valid",
        "valid-truncated",
        "valid-trailing",
        "wrong-message",
        "high-s-valid",
        "r-zero",
        "r-order",
        "s-zero",
        "s-order",
        "public-x-zero",
        "public-y-zero",
        "public-x-field-prime",
        "public-y-field-prime",
    }
    missing = needed - names
    if missing:
        fail(f"{label} omitted required cases: {sorted(missing)}")
    by_name = {row["name"]: row for row in rows}
    if by_name["valid"]["output"] != "00" * 31 + "01":
        fail(f"{label} valid signature did not return true")
    if by_name["high-s-valid"]["output"] != "00" * 31 + "01":
        fail(f"{label} high-S signature was not accepted")
    for name, row in by_name.items():
        if name not in ("valid", "high-s-valid") and row["output"] != "":
            fail(f"{label} invalid case returned true: {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()

    source = pathlib.Path(os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm"))
    exporter = (HERE / "p256_reference.go").read_bytes()
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-p256-") as directory:
            tree = pathlib.Path(directory)
            archive = checked_output(["git", "-C", str(source), "archive", revision])
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd/p256_reference"
            command.mkdir(parents=True)
            (command / "p256_reference.go").write_bytes(exporter)
            data = checked_output(
                ["go", "run", "-mod=readonly", "./cmd/p256_reference"], cwd=tree
            )
            validate_artifact(label, data)
            artifacts[label] = data

    if artifacts["public"] != artifacts["local"]:
        fail("pinned Go P-256 outputs disagree")

    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": checked_output(["go", "version"], text=True).strip(),
        "scope": (
            "Direct Cacti address 0x0100 RequiredGas/Run over exact-length, "
            "signature-scalar and public-key boundaries; no EVM admission, registry, "
            "fork activation, frame, state or production-routing claim"
        ),
        "sha256": {
            label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()
        },
        "exporter_sha256": hashlib.sha256(exporter).hexdigest(),
    }
    manifest_bytes = (json.dumps(manifest, indent=2) + "\n").encode()
    expected = {
        "public.json": artifacts["public"],
        "local.json": artifacts["local"],
        "manifest.json": manifest_bytes,
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
