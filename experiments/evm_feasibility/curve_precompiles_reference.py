#!/usr/bin/env python3
"""Reproduce the bounded dual-pin Go BN254/BLAKE2F corpus.

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
FIXTURES = HERE / "fixtures"
PREFIX = "curve_precompiles_reference"


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
    rows = document.get("curve_precompiles")
    if not isinstance(rows, list) or not rows:
        fail(f"{label} omitted curve_precompiles rows")
    names = set()
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            fail(f"{label} row {index} is not an object")
        required = {"name", "address", "input", "required_gas", "output", "error"}
        if set(row) != required:
            fail(f"{label} row {index} has unexpected fields")
        if row["name"] in names:
            fail(f"{label} duplicate row name: {row['name']}")
        names.add(row["name"])
        if row["address"] not in (6, 7, 8, 9):
            fail(f"{label} row {index} has unsupported address")
        for field in ("input", "output"):
            try:
                bytes.fromhex(row[field])
            except (TypeError, ValueError) as error:
                fail(f"{label} row {index} invalid {field}: {error}")
        if not isinstance(row["required_gas"], int) or row["required_gas"] < 0:
            fail(f"{label} row {index} invalid gas")
        if not isinstance(row["error"], str):
            fail(f"{label} row {index} invalid error")
    needed = {
        "add-empty-infinities",
        "add-off-curve-first",
        "add-out-of-field-first",
        "mul-truncated-scalar",
        "mul-trailing-ignored",
        "pairing-empty-true",
        "pairing-one-false",
        "pairing-negated-product-true",
        "pairing-bad-length-193",
        "pairing-invalid-g2",
        "blake-known-abc-final",
        "blake-zero-round-nonfinal",
        "blake-invalid-final",
        "blake-short-212",
    }
    missing = needed - names
    if missing:
        fail(f"{label} omitted required cases: {sorted(missing)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()

    source = pathlib.Path(os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm"))
    exporter = (HERE / f"{PREFIX}.go").read_bytes()
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-curve-precompiles-") as directory:
            tree = pathlib.Path(directory)
            archive = checked_output(["git", "-C", str(source), "archive", revision])
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd/curve_precompiles_reference"
            command.mkdir(parents=True)
            (command / f"{PREFIX}.go").write_bytes(exporter)
            data = checked_output(
                ["go", "run", "-mod=readonly", "./cmd/curve_precompiles_reference"], cwd=tree
            )
            validate_artifact(label, data)
            artifacts[label] = data

    if artifacts["public"] != artifacts["local"]:
        fail("pinned Go curve-precompile outputs disagree")

    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": checked_output(["go", "version"], text=True).strip(),
        "scope": (
            "Direct Californicum addresses 6..8 and Ficus address 9 RequiredGas/Run; "
            "bounded inputs, no EVM admission, activation, registry routing, or state claim"
        ),
        "max_blake2f_rounds": 12,
        "sha256": {
            label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()
        },
        "exporter_sha256": hashlib.sha256(exporter).hexdigest(),
    }
    manifest_path = FIXTURES / f"{PREFIX}_manifest.json"
    if args.record:
        FIXTURES.mkdir(exist_ok=True)
        for label, data in artifacts.items():
            (FIXTURES / f"{PREFIX}_{label}.json").write_bytes(data)
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        try:
            saved_manifest = json.loads(manifest_path.read_text())
        except (OSError, json.JSONDecodeError) as error:
            fail(f"cannot read saved manifest: {error}")
        if saved_manifest != manifest:
            fail("curve-precompile manifest mismatch")
        for label, data in artifacts.items():
            fixture_path = FIXTURES / f"{PREFIX}_{label}.json"
            try:
                saved_data = fixture_path.read_bytes()
            except OSError as error:
                fail(f"cannot read {label} fixture: {error}")
            if saved_data != data:
                fail(f"curve-precompile fixture mismatch: {label}")

    action = "recorded" if args.record else "reproduced"
    print(f"Both pinned curve-precompile references agree; {action}")


if __name__ == "__main__":
    main()
