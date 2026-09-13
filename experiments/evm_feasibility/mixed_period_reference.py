#!/usr/bin/env python3
"""Run and verify the initial mixed-period witness in its three supported modes."""

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
SOURCES = (
    "mixed_period_reference.go",
    "mixed_period_reference_public.go",
    "mixed_period_reference_local.go",
)


def run_reference(revision: str, observer: bool) -> bytes:
    with tempfile.TemporaryDirectory(prefix="rustaxa-mixed-period-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        command = tree / "cmd/mixed_period_reference"
        command.mkdir(parents=True)
        for name in SOURCES:
            (command / name).write_bytes((HERE / name).read_bytes())
        args = ["go", "run", "-mod=readonly"]
        if observer:
            args.append("-tags=concrete_observer")
        args.extend(["./cmd/mixed_period_reference", "--mode", "observer" if observer else "batched"])
        return subprocess.check_output(args, cwd=tree)


def require_equal(left, right, label: str) -> None:
    if left != right:
        raise RuntimeError(f"mixed-period modes disagree at {label}")


def compare_rows(left, right, label: str, allow_cf2_extras: bool = False) -> dict:
    if len(left) != 5 or len(right) != 5:
        raise RuntimeError(f"{label} must contain five concrete columns")
    for column in (0, 2, 3, 4):
        require_equal(left[column], right[column], f"{label}.CF{column + 1}")
    shared = set(left[1]) & set(right[1])
    for key in shared:
        require_equal(left[1][key], right[1][key], f"{label}.CF2[{key}]")
    left_only = sorted(set(left[1]) - set(right[1]))
    right_only = sorted(set(right[1]) - set(left[1]))
    if not allow_cf2_extras and (left_only or right_only):
        raise RuntimeError(f"{label}.CF2 key sets differ")
    return {
        "shared_cf2_rows": len(shared),
        "left_only_cf2_keys": left_only,
        "right_only_cf2_keys": right_only,
    }


def compare_runs(left: dict, right: dict, label: str, allow_final_cf2_extras: bool = False) -> dict:
    require_equal(left["configuration"], right["configuration"], f"{label}.configuration")
    require_equal(left["inputs"], right["inputs"], f"{label}.inputs")
    require_equal(left["genesis"]["root"], right["genesis"]["root"], f"{label}.genesis.root")
    require_equal(left["genesis"]["accounts"], right["genesis"]["accounts"], f"{label}.genesis.accounts")
    require_equal(
        left["genesis"]["native_storage_by_hashed_path"],
        right["genesis"]["native_storage_by_hashed_path"],
        f"{label}.genesis.native_storage",
    )
    require_equal(left["period"]["transaction"], right["period"]["transaction"], f"{label}.transaction")
    require_equal(left["period"]["reward_input"], right["period"]["reward_input"], f"{label}.reward_input")
    require_equal(left["period"]["reward_output"], right["period"]["reward_output"], f"{label}.reward_output")
    left_final = left["period"]["final"]
    right_final = right["period"]["final"]
    for field in ("root", "committed_root", "accounts", "native_storage_by_hashed_path"):
        require_equal(left_final[field], right_final[field], f"{label}.final.{field}")
    return {
        "final_semantics_and_roots_equal": True,
        "transaction_and_rewards_equal": True,
        "genesis_physical": compare_rows(left["genesis"]["rows"], right["genesis"]["rows"], f"{label}.genesis.rows"),
        "genesis_latest": compare_rows(left["genesis"]["latest_rows"], right["genesis"]["latest_rows"], f"{label}.genesis.latest"),
        "final_physical": compare_rows(
            left_final["rows"], right_final["rows"], f"{label}.final.rows", allow_final_cf2_extras
        ),
        "final_latest": compare_rows(
            left_final["latest_rows"], right_final["latest_rows"], f"{label}.final.latest", allow_final_cf2_extras
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()

    artifacts = {
        "mixed_public_batched.json": run_reference(REVISIONS["public"], False),
        "mixed_local_batched.json": run_reference(REVISIONS["local"], False),
        "mixed_local_observer.json": run_reference(REVISIONS["local"], True),
    }
    parsed = {name: json.loads(data) for name, data in artifacts.items()}
    public = parsed["mixed_public_batched.json"]
    local = parsed["mixed_local_batched.json"]
    observer = parsed["mixed_local_observer.json"]
    if public["observer_api"]["available"] or local["observer_api"]["available"]:
        raise RuntimeError("batched artifact unexpectedly advertises observer API")
    if not observer["observer_api"]["available"]:
        raise RuntimeError("local observer artifact does not advertise observer API")
    if "native_catalog" in public["genesis"] or "native_catalog" in local["genesis"]:
        raise RuntimeError("batched pin fabricated an unavailable concrete catalog")
    for phase in (observer["genesis"], observer["period"]["final"]):
        if "native_catalog" not in phase:
            raise RuntimeError("local observer omitted a concrete native catalog")

    comparison = {
        "schema": 1,
        "public_batched_vs_local_batched": compare_runs(public, local, "public_batched_vs_local_batched"),
        "local_batched_vs_local_observer": compare_runs(
            local, observer, "local_batched_vs_local_observer", allow_final_cf2_extras=True
        ),
        "limits": (
            "The public pin has no concrete observer API. CF2-only set differences are retained content-addressed "
            "nodes; all shared CF2 bytes, all other rows, final roots, execution, rewards, accounts, and complete "
            "native storage by hashed trie path must agree."
        ),
    }
    artifacts["mixed_comparison.json"] = (json.dumps(comparison, indent=2, sort_keys=True) + "\n").encode()
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"]).decode().strip(),
        "scope": (
            "Synthetic StateTransition genesis plus one signed fee/reward-bearing period; public/local batched and "
            "local concrete-observer comparison; incremental memory TrieSink rows, no RocksDB/header/production claim"
        ),
        "artifacts_sha256": {name: hashlib.sha256(data).hexdigest() for name, data in artifacts.items()},
        "source_sha256": {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest() for name in SOURCES},
        "runner_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
    }
    if args.record:
        FIXTURES.mkdir(exist_ok=True)
        for name, data in artifacts.items():
            (FIXTURES / name).write_bytes(data)
        (FIXTURES / "mixed_manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    else:
        manifest_path = FIXTURES / "mixed_manifest.json"
        if not manifest_path.exists():
            raise RuntimeError("mixed manifest is missing; run --record after review")
        saved = json.loads(manifest_path.read_text())
        if saved != manifest:
            raise RuntimeError("saved mixed metadata differs")
        for name, data in artifacts.items():
            path = FIXTURES / name
            if not path.exists() or path.read_bytes() != data:
                raise RuntimeError(f"saved {name} differs")
    print("Mixed witness " + ("recorded" if args.record else "reproduced") + " across public-batched/local-batched/local-observer")


if __name__ == "__main__":
    main()
