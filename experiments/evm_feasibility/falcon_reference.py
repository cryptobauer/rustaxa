#!/usr/bin/env python3
"""Reproduce Taraxa's dual-pin Cacti Falcon-512 precompile corpus."""

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = pathlib.Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures" / "falcon"


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
    if document.get("address") != "00" * 18 + "fa1c":
        fail(f"{label} selected the wrong Falcon address")
    if document.get("method_selector") != "de8f50a1":
        fail(f"{label} selected the wrong Falcon ABI method")
    if document.get("signature_size") != 666:
        fail(f"{label} changed the Falcon-512 signature size")
    if document.get("verifying_key_size") != 897:
        fail(f"{label} changed the Falcon-512 verifying-key size")
    rows = document.get("falcon")
    if not isinstance(rows, list) or not rows:
        fail(f"{label} omitted Falcon rows")
    names = set()
    required_fields = {
        "name",
        "input",
        "required_gas",
        "output",
        "error",
        "panic",
        "cryptographic_valid",
    }
    for index, row in enumerate(rows):
        if not isinstance(row, dict) or set(row) != required_fields:
            fail(f"{label} row {index} has unexpected fields")
        name = row["name"]
        if not isinstance(name, str) or name in names:
            fail(f"{label} row {index} has an invalid or duplicate name")
        names.add(name)
        try:
            input_bytes = bytes.fromhex(row["input"])
            output = bytes.fromhex(row["output"])
        except (TypeError, ValueError) as error:
            fail(f"{label} row {index} has invalid hex: {error}")
        expected_gas = 1465 + 6 * ((len(input_bytes) + 31) // 32)
        if row["required_gas"] != expected_gas:
            fail(f"{label} row {index} has an invalid gas quote")
        if (
            not isinstance(row["error"], str)
            or not isinstance(row["panic"], str)
            or not isinstance(row["cryptographic_valid"], bool)
        ):
            fail(f"{label} row {index} has invalid result metadata")
        if (row["error"] or row["panic"]) and output:
            fail(f"{label} row {index} returned output with an error or panic")
        if not row["error"] and not row["panic"] and output not in (
            bytes(32),
            bytes(31) + b"\x01",
        ):
            fail(f"{label} row {index} returned a noncanonical result word")
    needed = {
        "empty-input",
        "wrong-selector",
        "selector-only",
        "zero-signature-offset",
        "zero-key-offset",
        "zero-message-offset",
        "signature-offset-out-of-range",
        "key-offset-out-of-range",
        "message-offset-out-of-range",
        "zero-signature-length",
        "zero-key-length",
        "zero-message-length",
        "wrong-signature-length",
        "wrong-key-length",
        "truncated-signature",
        "truncated-key",
        "truncated-message",
        "historical-empty-message",
        "historical-valid",
        "historical-valid-long-message",
        "go-right-padded-message",
        "beyond-go-right-padding",
        "signed-message-length-tail",
        "wrapped-message-length-panic",
        "max-int-message-allocation-panic",
        "invalid-signature",
        "invalid-message",
        "reordered-fields",
        "unaligned-fields",
        "high-bits-offsets",
        "high-bits-lengths",
        "trailing-bytes",
    }
    missing = needed - names
    if missing:
        fail(f"{label} omitted required cases: {sorted(missing)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    source = pathlib.Path(
        os.environ.get("TARAXA_EVM_SOURCE", ROOT / "submodules/taraxa-evm")
    )
    exporter = (HERE / "falcon_reference.go").read_bytes()
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-falcon-") as directory:
            tree = pathlib.Path(directory)
            archive = checked_output(["git", "-C", str(source), "archive", revision])
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd/falcon_reference"
            command.mkdir(parents=True)
            (command / "falcon_reference.go").write_bytes(exporter)
            data = checked_output(
                ["go", "run", "-mod=readonly", "./cmd/falcon_reference"], cwd=tree
            )
            validate_artifact(label, data)
            artifacts[label] = data
    if artifacts["public"] != artifacts["local"]:
        fail("pinned Go Falcon outputs disagree")

    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": checked_output(["go", "version"], text=True).strip(),
        "scope": (
            "Direct Cacti Falcon-512 RequiredGas/Run behavior at exact address "
            "0xfa1c: selector errors, ABI offsets and lengths, historical crypto, "
            "empty message, invalid results and gas; no frame route or production claim"
        ),
        "sha256": {
            label: hashlib.sha256(data).hexdigest()
            for label, data in artifacts.items()
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
            fail(f"fixture differs: {path}; record only for an intentional update")


if __name__ == "__main__":
    main()
