#!/usr/bin/env python3
"""Run and verify the initial and four-period mixed-state witnesses."""

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
    "mixed_period_workload.go",
    "mixed_period_reference_public.go",
    "mixed_period_reference_local.go",
    "mixed_period_trace_disabled.go",
    "mixed_period_trace_enabled.go",
)

TRACE_TARGET = pathlib.Path("taraxa/state/state_evm/account.go")
TRACE_TARGET_SHA256 = "070a30bd1f0d8a34ca4e0037346f181a946944f8e38a25b32ec91e68d7577568"
TRACE_NEEDLE = b"func (self *Account) SetStateRawIrreversibly(key *common.Hash, value []byte) {\n"
TRACE_INSERTION = TRACE_NEEDLE + b"\tobserveMixedPeriodRawWrite(self.addr, *key, value)\n"
TRACE_HELPER = b'''package state_evm

import "github.com/Taraxa-project/taraxa-evm/common"

var mixedPeriodRawWriteObserver func(common.Address, common.Hash, []byte)

// SetMixedPeriodRawWriteObserver installs an archive-only synchronous observer.
func SetMixedPeriodRawWriteObserver(observer func(common.Address, common.Hash, []byte)) {
	mixedPeriodRawWriteObserver = observer
}

func observeMixedPeriodRawWrite(address common.Address, key common.Hash, value []byte) {
	if mixedPeriodRawWriteObserver != nil {
		mixedPeriodRawWriteObserver(address, key, common.CopyBytes(value))
	}
}
'''

REFUND_TRACE_TARGET = pathlib.Path("core/vm/evm.go")
REFUND_TRACE_TARGET_SHA256 = (
    "4389d46cc6fef3159c0da9fb3f548d9515aa555a2af85124824625c2c3f3e121",
    "9d63e0f58b3c2b32987f0979b88a30cccd294787eb7f5c8d45cc2ca1cbe5882f",
)
REFUND_TRACE_NEEDLE = b"\tgas_left += util.MinU64(self.state.GetRefund(), (gas_cap-gas_left)/2)\n"
REFUND_TRACE_INSERTION = b'''\trefund_counter := self.state.GetRefund()
\trefund_applied := util.MinU64(refund_counter, (gas_cap-gas_left)/2)
\tobserveMixedPeriodGasRefund(refund_counter, refund_applied)
\tgas_left += refund_applied
'''
REFUND_TRACE_HELPER = b'''package vm

var mixedPeriodGasRefundObserver func(uint64, uint64)

// SetMixedPeriodGasRefundObserver installs an archive-only synchronous observer.
func SetMixedPeriodGasRefundObserver(observer func(uint64, uint64)) {
\tmixedPeriodGasRefundObserver = observer
}

func observeMixedPeriodGasRefund(counter uint64, applied uint64) {
\tif mixedPeriodGasRefundObserver != nil {
\t\tmixedPeriodGasRefundObserver(counter, applied)
\t}
}
'''


def source_at_revision(revision: str, path: pathlib.Path) -> bytes:
    return subprocess.check_output(["git", "-C", str(SOURCE), "show", f"{revision}:{path}"])


def apply_raw_trace_patch(tree: pathlib.Path) -> None:
    target = tree / TRACE_TARGET
    source = target.read_bytes()
    if hashlib.sha256(source).hexdigest() != TRACE_TARGET_SHA256:
        raise RuntimeError("raw-write trace target hash differs")
    if source.count(TRACE_NEEDLE) != 1:
        raise RuntimeError("raw-write trace insertion point is not unique")
    target.write_bytes(source.replace(TRACE_NEEDLE, TRACE_INSERTION, 1))
    (target.parent / "mixed_period_raw_write_observer.go").write_bytes(TRACE_HELPER)
    refund_target = tree / REFUND_TRACE_TARGET
    refund_source = refund_target.read_bytes()
    if hashlib.sha256(refund_source).hexdigest() not in REFUND_TRACE_TARGET_SHA256:
        raise RuntimeError("gas-refund trace target hash differs")
    if refund_source.count(REFUND_TRACE_NEEDLE) != 1:
        raise RuntimeError("gas-refund trace insertion point is not unique")
    refund_target.write_bytes(refund_source.replace(REFUND_TRACE_NEEDLE, REFUND_TRACE_INSERTION, 1))
    (refund_target.parent / "mixed_period_gas_refund_observer.go").write_bytes(REFUND_TRACE_HELPER)


def run_reference(revision: str, observer: bool, scenario: str = "initial", traced: bool = False) -> bytes:
    with tempfile.TemporaryDirectory(prefix="rustaxa-mixed-period-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        if traced:
            apply_raw_trace_patch(tree)
        command = tree / "cmd/mixed_period_reference"
        command.mkdir(parents=True)
        for name in SOURCES:
            (command / name).write_bytes((HERE / name).read_bytes())
        args = ["go", "run", "-mod=readonly"]
        tags = []
        if observer:
            tags.append("concrete_observer")
        if traced:
            tags.append("mixed_raw_trace")
        if tags:
            args.append("-tags=" + ",".join(tags))
        args.extend(
            [
                "./cmd/mixed_period_reference",
                "--mode",
                "observer" if observer else "batched",
                "--scenario",
                scenario,
            ]
        )
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


def without_trace(value):
    if isinstance(value, dict):
        return {
            key: without_trace(item)
            for key, item in value.items()
            if key not in ("ordered_raw_writes", "raw_write_trace", "gas_refund", "gas_refund_trace")
        }
    if isinstance(value, list):
        return [without_trace(item) for item in value]
    return value


def without_observer(value):
    if isinstance(value, dict):
        return {key: without_observer(item) for key, item in value.items() if key != "observer"}
    if isinstance(value, list):
        return [without_observer(item) for item in value]
    return value


def compare_full_rows(left, right, label: str, allow_observer_retained_nodes: bool) -> dict:
    if len(left) != 5 or len(right) != 5:
        raise RuntimeError(f"{label} must contain five concrete columns")
    columns = []
    for column in range(5):
        shared = set(left[column]) & set(right[column])
        for key in shared:
            require_equal(left[column][key], right[column][key], f"{label}.CF{column + 1}[{key}]")
        left_only = sorted(set(left[column]) - set(right[column]))
        right_only = sorted(set(right[column]) - set(left[column]))
        if left_only:
            raise RuntimeError(f"{label}.CF{column + 1} has batched-only rows")
        if right_only and (not allow_observer_retained_nodes or column not in (1, 3)):
            raise RuntimeError(f"{label}.CF{column + 1} has unexpected observer-only rows")
        columns.append(
            {
                "column": column + 1,
                "shared_rows": len(shared),
                "batched_only_keys": left_only,
                "observer_only_retained_node_keys": right_only,
            }
        )
    return {"columns": columns}


def require_trace_equivalent(untraced: bytes, traced: bytes, label: str) -> dict:
    plain = json.loads(untraced)
    instrumented = json.loads(traced)
    require_equal(without_trace(plain), without_trace(instrumented), label + ".non_trace_facts")
    if plain["raw_write_trace"]["available"]:
        raise RuntimeError(label + " unmodified run advertises raw-write tracing")
    if not instrumented["raw_write_trace"]["available"]:
        raise RuntimeError(label + " traced run omitted raw-write tracing")
    if plain["gas_refund_trace"]["available"] or not instrumented["gas_refund_trace"]["available"]:
        raise RuntimeError(label + " gas-refund trace availability differs")
    if "ordered_raw_writes" not in instrumented["genesis"]:
        raise RuntimeError(label + " traced run omitted genesis raw writes")
    for period in instrumented["periods"]:
        if "ordered_raw_writes" not in period:
            raise RuntimeError(label + " traced run omitted terminal raw writes")
        for transaction in period["transactions"]:
            if "ordered_raw_writes" not in transaction:
                raise RuntimeError(label + " traced run omitted a transaction raw-write list")
            if "gas_refund" not in transaction:
                raise RuntimeError(label + " traced run omitted a transaction gas refund")
    return {
        "non_trace_facts_equal": True,
        "untraced_sha256": hashlib.sha256(untraced).hexdigest(),
        "traced_sha256": hashlib.sha256(traced).hexdigest(),
    }


def compare_full_runs(left: dict, right: dict, label: str, allow_observer_retained_nodes: bool) -> dict:
    for field in ("configuration", "inputs", "historical_reads", "reopen"):
        require_equal(left[field], right[field], f"{label}.{field}")
    for field in ("root", "accounts", "native_storage_by_hashed_path", "ordered_raw_writes"):
        require_equal(left["genesis"][field], right["genesis"][field], f"{label}.genesis.{field}")
    row_comparisons = {
        "genesis_physical": compare_full_rows(
            left["genesis"]["rows"], right["genesis"]["rows"], f"{label}.genesis.rows", False
        ),
        "genesis_latest": compare_full_rows(
            left["genesis"]["latest_rows"],
            right["genesis"]["latest_rows"],
            f"{label}.genesis.latest_rows",
            False,
        ),
        "periods": [],
    }
    require_equal(len(left["periods"]), len(right["periods"]), label + ".period_count")
    for index, (left_period, right_period) in enumerate(zip(left["periods"], right["periods"]), 1):
        period_label = f"{label}.period_{index}"
        for field in ("number", "planner_facts", "reward_input", "reward_output", "ordered_raw_writes"):
            require_equal(left_period[field], right_period[field], f"{period_label}.{field}")
        require_equal(
            without_observer(left_period["transactions"]),
            without_observer(right_period["transactions"]),
            period_label + ".transactions",
        )
        for field in ("root", "committed_root", "accounts", "native_storage_by_hashed_path"):
            require_equal(left_period["final"][field], right_period["final"][field], f"{period_label}.final.{field}")
        row_comparisons["periods"].append(
            {
                "number": index,
                "physical": compare_full_rows(
                    left_period["final"]["rows"],
                    right_period["final"]["rows"],
                    period_label + ".final.rows",
                    allow_observer_retained_nodes,
                ),
                "latest": compare_full_rows(
                    left_period["final"]["latest_rows"],
                    right_period["final"]["latest_rows"],
                    period_label + ".final.latest_rows",
                    allow_observer_retained_nodes,
                ),
            }
        )
    return {
        "execution_rewards_roots_and_semantics_equal": True,
        "ordered_raw_setter_operations_equal": True,
        "rows": row_comparisons,
    }


def validate_full_observer(observer: dict) -> None:
    expected = {
        2: {
            "set_commission_100": [(0, 1, 0, 20000, "")],
            "set_commission_wrong_owner": [(1, 0, 1, 20000, "This account is not owner of specified validator")],
            "set_commission_native_oog": [(2, 0, 1, 0, "out of gas")],
            "set_commission_200": [(3, 1, 0, 20000, "")],
            "set_commission_multi_300_400": [(4, 1, 0, 20000, ""), (5, 1, 0, 20000, "")],
            "set_commission_500_parent_revert": [(6, 2, 2, 20000, "")],
            "stateless_native_interleave_600_700": [(7, 2, 0, 20000, ""), (8, 2, 0, 20000, "")],
            "new_delegator_delegate_500": [(9, 0, 0, 40000, "")],
            "native_delegate_500_parent_revert": [(10, 1, 2, 40000, "")],
        },
        3: {
            "new_delegator_undelegate_v2_500": [(0, 0, 0, 60000, "")],
            "new_delegator_early_confirm": [(1, 0, 1, 20000, "Undelegation is not yet ready to be withdrawn")],
            "native_undelegate_v2_500": [(2, 1, 0, 60000, "")],
            "native_early_confirm_parent_revert": [(3, 1, 1, 20000, "Undelegation is not yet ready to be withdrawn")],
        },
        4: {
            "new_delegator_confirm": [(0, 0, 0, 20000, "")],
            "native_confirm_parent_revert": [(1, 1, 2, 20000, "")],
            "native_repeat_confirm_missing": [(2, 1, 1, 20000, "Undelegation does not exist")],
        },
    }
    for period in observer["periods"]:
        expected_period = expected.get(period["number"], {})
        transaction_invocations = []
        for transaction in period["transactions"]:
            invocations = transaction["observer"]["catalog"]["invocations"]
            transaction_invocations.extend(invocations)
            wanted = expected_period.get(transaction["name"], [])
            facts = [
                (item["Sequence"], item["Depth"], item["Disposition"], item["GasUsed"], item["Error"])
                for item in invocations
            ]
            require_equal(facts, wanted, f"observer.period_{period['number']}.{transaction['name']}.invocations")
        require_equal(
            period["final"]["native_catalog"]["invocations"],
            transaction_invocations,
            f"observer.period_{period['number']}.final_native_invocations_once",
        )
    statuses = {
        transaction["name"]: transaction["status"]
        for period in observer["periods"]
        for transaction in period["transactions"]
    }
    expected_raw_write_counts = {
        "set_commission_100": 1,
        "set_commission_wrong_owner": 0,
        "set_commission_native_oog": 0,
        "set_commission_200": 1,
        "set_commission_multi_300_400": 2,
        "set_commission_500_parent_revert": 1,
        "stateless_native_interleave_600_700": 2,
        "new_delegator_delegate_500": 8,
        "native_delegate_500_parent_revert": 7,
        "new_delegator_undelegate_v2_500": 17,
        "new_delegator_early_confirm": 0,
        "native_undelegate_v2_500": 16,
        "native_early_confirm_parent_revert": 0,
        "new_delegator_confirm": 8,
        "native_confirm_parent_revert": 8,
        "native_repeat_confirm_missing": 0,
    }
    require_equal(len(observer["genesis"]["ordered_raw_writes"]), 18, "observer.genesis.raw_write_count")
    require_equal(
        observer["genesis"]["native_catalog"]["coverage"]["all_live_dpos_storage_covered"],
        True,
        "observer.genesis.native_catalog.coverage",
    )
    for period in observer["periods"]:
        expected_terminal_counts = {
            1: {"begin_block": 0, "rewards": 3, "end_block": 0},
            2: {"begin_block": 0, "rewards": 3, "end_block": 2},
            3: {"begin_block": 0, "rewards": 3, "end_block": 2},
            4: {"begin_block": 0, "rewards": 3, "end_block": 0},
        }
        require_equal(
            {name: len(writes) for name, writes in period["ordered_raw_writes"].items()},
            expected_terminal_counts[period["number"]],
            f"observer.period_{period['number']}.terminal_raw_write_counts",
        )
        require_equal(
            period["final"]["native_catalog"]["coverage"]["all_live_dpos_storage_covered"],
            True,
            f"observer.period_{period['number']}.native_catalog.coverage",
        )
        for transaction in period["transactions"]:
            expected_count = expected_raw_write_counts.get(transaction["name"], 0)
            require_equal(
                len(transaction["ordered_raw_writes"]),
                expected_count,
                "observer.raw_write_count." + transaction["name"],
            )
            expected_refund = (
                {"counter": 24000, "applied": 13044}
                if transaction["name"] == "delete_lifecycle"
                else {"counter": 0, "applied": 0}
            )
            require_equal(transaction["gas_refund"], expected_refund, "observer.gas_refund." + transaction["name"])
    for name in (
        "parent_revert_child_selfdestruct",
        "set_commission_wrong_owner",
        "set_commission_native_oog",
        "set_commission_500_parent_revert",
        "native_delegate_500_parent_revert",
        "new_delegator_early_confirm",
        "native_early_confirm_parent_revert",
        "native_confirm_parent_revert",
    ):
        require_equal(statuses[name], 0, "observer.status." + name)
    require_equal(statuses["native_repeat_confirm_missing"], 1, "observer.status.native_repeat_confirm_missing")


def full_artifacts() -> tuple[dict[str, bytes], dict]:
    modes = {
        "mixed_workload_public_batched.json": (REVISIONS["public"], False),
        "mixed_workload_local_batched.json": (REVISIONS["local"], False),
        "mixed_workload_local_observer.json": (REVISIONS["local"], True),
    }
    untraced = {name: run_reference(revision, observer, "full") for name, (revision, observer) in modes.items()}
    traced = {
        name: run_reference(revision, observer, "full", traced=True)
        for name, (revision, observer) in modes.items()
    }
    trace_equivalence = {
        name: require_trace_equivalent(untraced[name], traced[name], name) for name in modes
    }
    parsed = {name: json.loads(data) for name, data in traced.items()}
    public = parsed["mixed_workload_public_batched.json"]
    local = parsed["mixed_workload_local_batched.json"]
    observer = parsed["mixed_workload_local_observer.json"]
    validate_full_observer(observer)
    if "native_catalog" in public["genesis"] or "native_catalog" in local["genesis"]:
        raise RuntimeError("full batched pin fabricated an unavailable native catalog")
    if "native_catalog" not in observer["genesis"]:
        raise RuntimeError("full observer omitted the genesis native catalog")
    for period in observer["periods"]:
        if period["period_catalog_identities"] is None or period["final"]["native_catalog"] is None:
            raise RuntimeError("full observer omitted a period native catalog")
        for transaction in period["transactions"]:
            if "observer" not in transaction:
                raise RuntimeError("full observer omitted an intermediate transaction observation")
    comparison = {
        "schema": 1,
        "trace_equivalence": trace_equivalence,
        "public_batched_vs_local_batched": compare_full_runs(
            public, local, "public_batched_vs_local_batched", False
        ),
        "local_batched_vs_local_observer": compare_full_runs(
            local, observer, "local_batched_vs_local_observer", True
        ),
        "limits": (
            "The raw-write hook observes synchronous SetStateRawIrreversibly setter-entry order, including repeated "
            "keys and empty tombstones. It does not establish physical persistence order or universal raw-state "
            "irreversibility. The public pin has no concrete observer API; observer-only CF2 main-trie and CF4 "
            "account-storage-trie keys are named retained content-addressed nodes. All shared CF2/CF4 bytes and all "
            "CF1/CF3/CF5 rows must agree."
        ),
    }
    artifacts = dict(traced)
    artifacts["mixed_workload_comparison.json"] = (
        json.dumps(comparison, indent=2, sort_keys=True) + "\n"
    ).encode()
    return artifacts, {name: hashlib.sha256(data).hexdigest() for name, data in untraced.items()}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    parser.add_argument("--scenario", choices=("initial", "full"), default="initial")
    args = parser.parse_args()

    if args.scenario == "full":
        artifacts, untraced_hashes = full_artifacts()
        manifest = {
            "schema": 1,
            "references": REVISIONS,
            "go_version": subprocess.check_output(["go", "version"]).decode().strip(),
            "scope": (
                "Four signed periods covering storage lifecycle, nested rollback, consensus-native and stateless "
                "calls, delegation custody, rewards, and close/reopen continuation; incremental memory TrieSink "
                "rows, no RocksDB, application header, production route, or reconstructed-root claim"
            ),
            "artifacts_sha256": {name: hashlib.sha256(data).hexdigest() for name, data in artifacts.items()},
            "untraced_artifacts_sha256": untraced_hashes,
            "source_sha256": {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest() for name in SOURCES},
            "runner_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            "raw_write_trace_patch": {
                "target": str(TRACE_TARGET),
                "original_sha256": TRACE_TARGET_SHA256,
                "needle_sha256": hashlib.sha256(TRACE_NEEDLE).hexdigest(),
                "needle_required_count": 1,
                "insertion_sha256": hashlib.sha256(TRACE_INSERTION).hexdigest(),
                "patched_target_sha256": hashlib.sha256(
                    source_at_revision(REVISIONS["public"], TRACE_TARGET).replace(TRACE_NEEDLE, TRACE_INSERTION, 1)
                ).hexdigest(),
                "injected_helper_sha256": hashlib.sha256(TRACE_HELPER).hexdigest(),
            },
            "gas_refund_trace_patch": {
                "target": str(REFUND_TRACE_TARGET),
                "original_sha256": {
                    name: hashlib.sha256(source_at_revision(revision, REFUND_TRACE_TARGET)).hexdigest()
                    for name, revision in REVISIONS.items()
                },
                "allowed_original_sha256": list(REFUND_TRACE_TARGET_SHA256),
                "needle_sha256": hashlib.sha256(REFUND_TRACE_NEEDLE).hexdigest(),
                "needle_required_count": 1,
                "insertion_sha256": hashlib.sha256(REFUND_TRACE_INSERTION).hexdigest(),
                "patched_target_sha256": {
                    name: hashlib.sha256(
                        source_at_revision(revision, REFUND_TRACE_TARGET).replace(
                            REFUND_TRACE_NEEDLE, REFUND_TRACE_INSERTION, 1
                        )
                    ).hexdigest()
                    for name, revision in REVISIONS.items()
                },
                "injected_helper_sha256": hashlib.sha256(REFUND_TRACE_HELPER).hexdigest(),
            },
        }
        manifest_path = FIXTURES / "mixed_workload_manifest.json"
        if args.record:
            FIXTURES.mkdir(exist_ok=True)
            for name, data in artifacts.items():
                (FIXTURES / name).write_bytes(data)
            manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        else:
            if not manifest_path.exists():
                raise RuntimeError("mixed workload manifest is missing; run --scenario full --record after review")
            if json.loads(manifest_path.read_text()) != manifest:
                raise RuntimeError("saved mixed workload metadata differs")
            for name, data in artifacts.items():
                path = FIXTURES / name
                if not path.exists() or path.read_bytes() != data:
                    raise RuntimeError(f"saved {name} differs")
        print(
            "Full mixed workload "
            + ("recorded" if args.record else "reproduced")
            + " across traced/untraced public-batched, local-batched, and local-observer"
        )
        return

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
