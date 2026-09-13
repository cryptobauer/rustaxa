#!/usr/bin/env python3
"""Reproduce the bounded dual-pin Taraxa BLS12-381 corpus.

The default mode verifies checked-in bytes and hashes. ``--record`` replaces
only ``fixtures/bls`` after both immutable Go revisions agree byte-for-byte.
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
FIXTURES = HERE / "fixtures" / "bls"
EXPORTER = HERE / "bls_reference.go"


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
    rows = document.get("bls")
    if not isinstance(rows, list) or not rows:
        fail(f"{label} omitted BLS rows")
    names = set()
    seen_operations = set()
    seen_addresses = {"ficus": set(), "cacti": set()}
    for index, row in enumerate(rows):
        required = {
            "name",
            "registry",
            "operation",
            "address",
            "input",
            "required_gas",
            "output",
            "error",
        }
        if not isinstance(row, dict) or set(row) not in (required, required | {"repeat"}):
            fail(f"{label} row {index} has unexpected shape")
        identity = (row["registry"], row["name"])
        if identity in names:
            fail(f"{label} duplicate row: {identity}")
        names.add(identity)
        if row["registry"] not in seen_addresses:
            fail(f"{label} row {index} has unknown registry")
        if not isinstance(row["address"], int) or not 11 <= row["address"] <= 19:
            fail(f"{label} row {index} has invalid address")
        seen_operations.add(row["operation"])
        seen_addresses[row["registry"]].add(row["address"])
        for field in ("input", "output"):
            try:
                bytes.fromhex(row[field])
            except (TypeError, ValueError) as error:
                fail(f"{label} row {index} invalid {field}: {error}")
        if "repeat" in row:
            repeat = row["repeat"]
            if row["input"] != "" or not isinstance(repeat, dict):
                fail(f"{label} row {index} invalid repeated input")
            if set(repeat) != {"element", "count"} or repeat["count"] not in (128, 129):
                fail(f"{label} row {index} invalid repeat descriptor")
            try:
                element = bytes.fromhex(repeat["element"])
            except (TypeError, ValueError) as error:
                fail(f"{label} row {index} invalid repeat element: {error}")
            if len(element) not in (160, 288):
                fail(f"{label} row {index} invalid repeat element length")
        if not isinstance(row["required_gas"], int) or row["required_gas"] < 0:
            fail(f"{label} row {index} invalid gas")
        if not isinstance(row["error"], str):
            fail(f"{label} row {index} invalid error")

    expected_operations = {
        "g1_add",
        "g1_mul",
        "g1_multiexp",
        "g2_add",
        "g2_mul",
        "g2_multiexp",
        "pairing",
        "map_g1",
        "map_g2",
    }
    if seen_operations != expected_operations:
        fail(f"{label} operation coverage mismatch: {sorted(seen_operations)}")
    if seen_addresses["ficus"] != set(range(11, 20)):
        fail(f"{label} Ficus address coverage mismatch")
    if seen_addresses["cacti"] != set(range(11, 18)):
        fail(f"{label} Cacti address coverage mismatch")
    needed = {
        "g1-add-non-subgroup",
        "g1-multiexp-non-subgroup-one",
        "g1-multiexp-k128-discount",
        "g1-multiexp-k129-cap",
        "g2-add-non-subgroup",
        "g2-multiexp-non-subgroup-one",
        "g2-multiexp-k128-discount",
        "g2-multiexp-k129-cap",
        "pairing-g1-non-subgroup",
        "pairing-g2-non-subgroup",
        "pairing-infinity-true",
        "map-g1-noncanonical-field",
        "map-g2-second-bad-top",
    }
    missing = needed - {name for _, name in names}
    if missing:
        fail(f"{label} omitted required cases: {sorted(missing)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()

    source = pathlib.Path(os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm"))
    exporter = EXPORTER.read_bytes()
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-bls-") as directory:
            tree = pathlib.Path(directory)
            archive = checked_output(["git", "-C", str(source), "archive", revision])
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd" / "bls_reference"
            command.mkdir(parents=True)
            (command / EXPORTER.name).write_bytes(exporter)
            data = checked_output(
                ["go", "run", "-mod=readonly", "./cmd/bls_reference"], cwd=tree
            )
            validate_artifact(label, data)
            artifacts[label] = data

    if artifacts["public"] != artifacts["local"]:
        fail("pinned Go BLS outputs disagree")

    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": checked_output(["go", "version"], text=True).strip(),
        "scope": (
            "Direct Ficus 11..19 and Cacti 11..17 BLS RequiredGas/Run; "
            "bounded arithmetic, length, field, subgroup, infinity and MSM discount cases; "
            "no EVM admission, historical classifier, frame settlement, state or routing claim"
        ),
        "large_msm": (
            "128 and 129 infinity-point/zero-scalar pairs only; rows store the exact "
            "element plus repeat count instead of duplicating zero bytes"
        ),
        "sha256": {
            label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()
        },
        "exporter_sha256": hashlib.sha256(exporter).hexdigest(),
    }
    manifest_path = FIXTURES / "manifest.json"
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for label, data in artifacts.items():
            (FIXTURES / f"{label}.json").write_bytes(data)
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        try:
            saved_manifest = json.loads(manifest_path.read_text())
        except (OSError, json.JSONDecodeError) as error:
            fail(f"cannot read saved manifest: {error}")
        if saved_manifest != manifest:
            fail("BLS manifest mismatch")
        for label, data in artifacts.items():
            try:
                saved_data = (FIXTURES / f"{label}.json").read_bytes()
            except OSError as error:
                fail(f"cannot read {label} fixture: {error}")
            if saved_data != data:
                fail(f"BLS fixture mismatch: {label}")

    action = "recorded" if args.record else "reproduced"
    print(f"Both pinned BLS references agree; {action}")


if __name__ == "__main__":
    main()
