#!/usr/bin/env python3
"""Reproduce pinned cross-period accrued-reward cancellation fixtures; use --record to update."""

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
GO_SOURCE = HERE / "native_cancel_reward_reference.go"
SUPPORT_SOURCE = HERE / "native_simulation_reference.go"
V1_SUPPORT_SOURCE = HERE / "native_v1_custody_reference.go"
FIXTURES = HERE / "fixtures/native_cancel_reward"
TRACE_TARGET = pathlib.Path("taraxa/state/state_evm/account.go")
TRACE_SHA256 = "070a30bd1f0d8a34ca4e0037346f181a946944f8e38a25b32ec91e68d7577568"
TRACE_NEEDLE = b"func (self *Account) SetStateRawIrreversibly(key *common.Hash, value []byte) {\n"
TRACE_INSERTION = TRACE_NEEDLE + b"\tobserveNativeV1CustodyRawWrite(self.addr, *key, value)\n"
TRACE_HELPER = b'''package state_evm

import "github.com/Taraxa-project/taraxa-evm/common"

var nativeV1CustodyRawWriteObserver func(common.Address, common.Hash, []byte)

// SetNativeV1CustodyRawWriteObserver installs the archive-only synchronous
// observer used to record calls at the irreversible raw-storage boundary.
func SetNativeV1CustodyRawWriteObserver(observer func(common.Address, common.Hash, []byte)) {
	nativeV1CustodyRawWriteObserver = observer
}

func observeNativeV1CustodyRawWrite(address common.Address, key common.Hash, value []byte) {
	if nativeV1CustodyRawWriteObserver != nil {
		nativeV1CustodyRawWriteObserver(address, key, common.CopyBytes(value))
	}
}
'''


def run_reference(revision: str) -> bytes:
    with tempfile.TemporaryDirectory(prefix="rustaxa-native-cancel-reward-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        target = tree / TRACE_TARGET
        source = target.read_bytes()
        if hashlib.sha256(source).hexdigest() != TRACE_SHA256 or source.count(TRACE_NEEDLE) != 1:
            raise RuntimeError("cancellation reward raw-write trace target changed")
        target.write_bytes(source.replace(TRACE_NEEDLE, TRACE_INSERTION, 1))
        (target.parent / "native_v1_custody_observer.go").write_bytes(TRACE_HELPER)
        command = tree / "cmd/native_cancel_reward_reference"
        command.mkdir(parents=True)
        support = SUPPORT_SOURCE.read_text().replace(
            "func main() {", "func nativeSimulationSupportMain() {", 1
        )
        v1_support = V1_SUPPORT_SOURCE.read_text().replace(
            "func main() {", "func nativeV1CustodySupportMain() {", 1
        )
        (command / "native_simulation_support.go").write_text(support)
        (command / "native_v1_custody_support.go").write_text(v1_support)
        (command / GO_SOURCE.name).write_bytes(GO_SOURCE.read_bytes())
        return subprocess.check_output(
            ["go", "run", "-mod=readonly", "./cmd/native_cancel_reward_reference"],
            cwd=tree,
        )


def validate(label: str, data: bytes) -> dict:
    document = json.loads(data)
    if document["schema"] != 1:
        raise RuntimeError(f"{label}: unsupported schema")
    if document["configuration"] != {
        "yield_percentage": 20,
        "blocks_per_year": 1,
        "commission": 100,
        "magnolia": 0,
        "ficus": 0,
        "cornus": 0,
        "cacti": "disabled",
    }:
        raise RuntimeError(f"{label}: cancellation reward configuration changed")
    scenarios = {scenario["name"]: scenario for scenario in document["scenarios"]}
    if set(scenarios) != {
        "v1_accrued_reward",
        "v2_accrued_reward",
        "v1_same_period_reward",
        "v2_same_period_reward",
    }:
        raise RuntimeError(f"{label}: cancellation reward scenarios changed")
    cases = (
        ("v1_accrued_reward", "v1", "separate_reward_period", 2, 3),
        ("v2_accrued_reward", "v2", "separate_reward_period", 2, 3),
        ("v1_same_period_reward", "v1", "undelegate_then_reward_same_period", 1, 2),
        ("v2_same_period_reward", "v2", "undelegate_then_reward_same_period", 1, 2),
    )
    for name, version, timing, reward_period, cancel_period in cases:
        scenario = scenarios[name]
        transactions = scenario["transactions"]
        if (
            scenario["version"] != version
            or scenario["timing"] != timing
            or scenario["amount"] != "300"
            or len(transactions) != 2
            or transactions[0]["consensus_error"]
            or transactions[1]["consensus_error"]
            or transactions[0]["execution_error"]
            or transactions[1]["execution_error"]
            or scenario["reward_minted"] != "140"
        ):
            raise RuntimeError(f"{label}/{name}: lifecycle changed")
        cancel = transactions[1]
        expected_cancel_name = f"cancel_{version}"
        expected_undelegate_selector = "4d99dd16" if version == "v1" else "bd0e7fcc"
        expected_cancel_selector = "399ff554" if version == "v1" else "b6e1e329"
        expected_cancel_topic = (
            "fc25f8a919d19f2c2dfce21115718abc9ef2b1e0c9218a488f614c75be4184b7"
            if version == "v1"
            else "e0474558d9b6ee7a45f2d6d12effd21909b53360eb73eda6cf0f197031738fee"
        )
        if (
            cancel["name"] != expected_cancel_name
            or cancel["nonce"] != "1"
            or transactions[0]["selector"] != expected_undelegate_selector
            or cancel["selector"] != expected_cancel_selector
            or cancel["gas_used"] != (81_464 if version == "v1" else 81_656)
            or len(cancel["logs"]) != 2
            or len(cancel["ordered_raw_writes"]) != (10 if version == "v1" else 13)
            or cancel["logs"][0]["topics"][0]
            != "9310ccfcb8de723f578a9e4282ea9f521f05ae40dc08f3068dfad528a65ee3c7"
            or int(cancel["logs"][0]["data"], 16) != 138
            or cancel["logs"][1]["topics"][0] != expected_cancel_topic
            or int(cancel["logs"][1]["data"], 16) != 300
        ):
            raise RuntimeError(f"{label}/{name}: accrued cancellation changed")
        if scenario["reward_ordered_raw_writes"] != [
            {
                "address": "00" * 19 + "fe",
                "key": "18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c",
                "value": "c3818b01",
            },
            {
                "address": "00" * 19 + "fe",
                "key": "d0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873",
                "value": "8c",
            },
        ]:
            raise RuntimeError(f"{label}/{name}: reward raw writes changed")
        before = scenario["before"]
        after_reward = scenario["after_reward"]
        after_cancel = scenario["after_cancel"]
        if (
            before["period"] != 0
            or after_reward["period"] != reward_period
            or after_cancel["period"] != cancel_period
            or after_reward["validator_stake"] != "700"
            or after_cancel["validator_stake"] != "1000"
            or after_cancel["total_delegated"] != "1000"
            or after_reward["contract_balance"] != "1140"
            or after_cancel["delegator_balance"] != "1138"
            or after_cancel["contract_balance"] != "1002"
        ):
            raise RuntimeError(f"{label}/{name}: state progression changed")
        if timing == "separate_reward_period":
            after_undelegate = scenario.get("after_undelegate")
            if (
                after_undelegate is None
                or after_undelegate["period"] != 1
                or after_undelegate["validator_stake"] != "700"
                or len(scenario["undelegate_end_block_ordered_raw_writes"]) != 2
            ):
                raise RuntimeError(f"{label}/{name}: separate-period checkpoint changed")
            if scenario["reward_end_block_ordered_raw_writes"] is not None:
                raise RuntimeError(f"{label}/{name}: separate reward EndBlock changed")
        elif (
            "after_undelegate" in scenario
            or scenario["undelegate_end_block_ordered_raw_writes"] is not None
            or len(scenario["reward_end_block_ordered_raw_writes"]) != 2
        ):
            raise RuntimeError(f"{label}/{name}: same-period witness gained a false checkpoint")
    return document


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    for label, data in artifacts.items():
        validate(label, data)
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("pinned cancellation reward references diverged")
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"], text=True).strip(),
        "scope": (
            "One live actual StateTransition per V1/V2 witness, retaining both P1 undelegation, "
            "P2 reward, P3 cancellation and P1 undelegation+reward, P2 cancellation timings; "
            "all include ordered raw writes and logs; "
            "no injected underfunding, Rust publication, or reopen parity is claimed"
        ),
        "instrumentation": {
            "target": str(TRACE_TARGET),
            "sha256": TRACE_SHA256,
            "observer_sha256": hashlib.sha256(TRACE_HELPER).hexdigest(),
            "insertion": "synchronous copy-only observer before irreversible raw mutation body",
        },
        "support_sha256": hashlib.sha256(SUPPORT_SOURCE.read_bytes()).hexdigest(),
        "v1_support_sha256": hashlib.sha256(V1_SUPPORT_SOURCE.read_bytes()).hexdigest(),
        "public_local_byte_identical": artifacts["public"] == artifacts["local"],
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "exporter_sha256": hashlib.sha256(GO_SOURCE.read_bytes()).hexdigest(),
        "harness_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
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
            "instrumentation",
            "support_sha256",
            "v1_support_sha256",
            "public_local_byte_identical",
            "sha256",
            "exporter_sha256",
            "harness_sha256",
        ):
            if saved[field] != manifest[field]:
                raise RuntimeError(f"cancellation reward manifest mismatch: {field}")
        for label, data in artifacts.items():
            if data != (FIXTURES / f"{label}.json").read_bytes():
                raise RuntimeError(f"cancellation reward fixture mismatch: {label}")
    print("Both pinned cancellation reward references " + ("recorded" if args.record else "reproduced"))
    print("Public/local byte-identical:", manifest["public_local_byte_identical"])


if __name__ == "__main__":
    main()
