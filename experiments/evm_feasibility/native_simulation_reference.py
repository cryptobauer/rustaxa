#!/usr/bin/env python3
"""Reproduce pinned DryRunner native-call fixtures; use --record to update."""

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
GO_SOURCE = HERE / "native_simulation_reference.go"
FIXTURES = HERE / "fixtures/native_simulation"


def run_reference(revision: str) -> bytes:
    with tempfile.TemporaryDirectory(prefix="rustaxa-native-simulation-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        command = tree / "cmd/native_simulation_reference"
        command.mkdir(parents=True)
        (command / GO_SOURCE.name).write_bytes(GO_SOURCE.read_bytes())
        return subprocess.check_output(
            ["go", "run", "-mod=readonly", "./cmd/native_simulation_reference"], cwd=tree
        )


def validate(label: str, data: bytes) -> dict:
    document = json.loads(data)
    if document["schema"] != 1:
        raise RuntimeError(f"{label}: unsupported schema")
    if not document["repeat_identical"] or not document["committed_state_unchanged"]:
        raise RuntimeError(f"{label}: DryRunner leaked or repeated output diverged")
    current = document["state_before"]["current"]
    delayed = document["state_before"]["delayed"]
    if current != {"eligible": True, "total_eligible_votes": 11, "validator_eligible_votes": 11}:
        raise RuntimeError(f"{label}: unexpected current eligibility: {current}")
    if delayed != {"effective_period": 0, "eligible": False, "total_eligible_votes": 0, "validator_eligible_votes": 0}:
        raise RuntimeError(f"{label}: unexpected delayed eligibility: {delayed}")
    cases = {case["name"]: case for case in document["cases"]}
    expected = {
        "delegate_then_current_and_delayed_queries",
        "repeat_delegate_then_queries",
        "malformed_get_total_delegation",
        "missing_get_validator",
        "short_get_delegations",
        "high_bits_get_total_delegation",
    }
    if set(cases) != expected:
        raise RuntimeError(f"{label}: case set changed")
    successful = cases["delegate_then_current_and_delayed_queries"]["output"]
    if successful["consensus_error"] or successful["execution_error"]:
        raise RuntimeError(f"{label}: wrapper call failed: {successful}")
    if successful["return"] != f'{25:064x}{0:064x}':
        raise RuntimeError(f"{label}: staged/delayed query words changed")
    if not successful["logs"]:
        raise RuntimeError(f"{label}: delegate emitted no log")
    delegated = successful["logs"][0]
    if (
        delegated["address"] != "00" * 19 + "fe"
        or len(delegated["topics"]) != 3
        or any(len(topic) != 64 for topic in delegated["topics"])
        or delegated["data"] != f"{25:064x}"
    ):
        raise RuntimeError(f"{label}: typed Delegated log changed")
    if successful != cases["repeat_delegate_then_queries"]["output"]:
        raise RuntimeError(f"{label}: repeated wrapper result differs")
    expected_errors = {
        "malformed_get_total_delegation": (
            "abi: cannot marshal in to go type: length insufficient 0 require 32"
        ),
        "missing_get_validator": "Validator does not exist",
        "short_get_delegations": (
            "abi: cannot marshal in to go type: length insufficient 32 require 64"
        ),
    }
    for name, expected_error in expected_errors.items():
        if cases[name]["output"]["execution_error"] != expected_error:
            raise RuntimeError(f"{label}: {name} error changed")
    if cases["malformed_get_total_delegation"]["reference_stdout"] != (
        "Unable to parse getTotalDelegation input args:  "
        + expected_errors["malformed_get_total_delegation"]
        + "\n"
    ):
        raise RuntimeError(f"{label}: malformed-query source diagnostic changed")
    if cases["short_get_delegations"]["reference_stdout"] != (
        "Unable to parse getDelegations input args:  "
        + expected_errors["short_get_delegations"]
        + "\n"
    ):
        raise RuntimeError(f"{label}: short-query source diagnostic changed")
    high_bits = cases["high_bits_get_total_delegation"]["output"]
    if high_bits["consensus_error"] or high_bits["execution_error"] or high_bits["return"] != f'{0:064x}':
        raise RuntimeError(f"{label}: high address bits changed Go ABI behavior: {high_bits}")
    effective = {case["output"]["effective_nonce"] for case in cases.values()}
    if len(effective) != 1 or len(next(iter(effective))) < 80:
        raise RuntimeError(f"{label}: full-width persisted nonce was not applied")
    if not document["state_before"]["seed_rows"]:
        raise RuntimeError(f"{label}: no replayable seed rows")
    return document


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    parsed = {label: validate(label, data) for label, data in artifacts.items()}
    public_cases = {case["name"]: case for case in parsed["public"]["cases"]}
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"], text=True).strip(),
        "scope": (
            "Actual state_dry_runner.DryRunner.Apply at synthetic historical H=1 over versioned trie state: "
            "ordinary wrapper delegate plus current/delayed DPoS queries, repeated disposal, malformed and "
            "missing-record errors, full-width nonce, gas/log/output observations; no RPC or production routing claim"
        ),
        "fixture_schema": {
            "state_before/state_after.seed_rows": "column plus physical key/value, including period suffixes for versioned trie columns",
            "cases[].output": "exact DryRunner effective nonce, gas, consensus/execution errors, return bytes and typed hex address/topics/data logs",
            "current/delayed": "DPoS facts at H and effective H-delegation_delay",
        },
        "public_local_byte_identical": artifacts["public"] == artifacts["local"],
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "exporter_sha256": hashlib.sha256(GO_SOURCE.read_bytes()).hexdigest(),
        "exact_errors": {
            name: case["output"]["execution_error"]
            for name, case in public_cases.items()
            if case["output"]["execution_error"]
        },
    }
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for label, data in artifacts.items():
            (FIXTURES / f"{label}.json").write_bytes(data)
        (FIXTURES / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        saved = json.loads((FIXTURES / "manifest.json").read_text())
        for field in (
            "schema",
            "references",
            "scope",
            "fixture_schema",
            "public_local_byte_identical",
            "sha256",
            "exporter_sha256",
            "exact_errors",
        ):
            if saved[field] != manifest[field]:
                raise RuntimeError(f"native simulation manifest mismatch: {field}")
        for label, data in artifacts.items():
            if data != (FIXTURES / f"{label}.json").read_bytes():
                raise RuntimeError(f"native simulation fixture mismatch: {label}")
    print("Both pinned DryRunner native simulation references " + ("recorded" if args.record else "reproduced"))
    print("Public/local byte-identical:", manifest["public_local_byte_identical"])


if __name__ == "__main__":
    main()
