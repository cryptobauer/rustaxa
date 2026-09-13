#!/usr/bin/env python3
"""Reproduce pinned V1/V2 cancellation custody fixtures; use --record to update."""

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
GO_SOURCE = HERE / "native_cancel_custody_reference.go"
SUPPORT_SOURCE = HERE / "native_simulation_reference.go"
V1_SUPPORT_SOURCE = HERE / "native_v1_custody_reference.go"
FIXTURES = HERE / "fixtures/native_cancel_custody"
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
    with tempfile.TemporaryDirectory(prefix="rustaxa-native-cancel-custody-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        target = tree / TRACE_TARGET
        source = target.read_bytes()
        if hashlib.sha256(source).hexdigest() != TRACE_SHA256 or source.count(TRACE_NEEDLE) != 1:
            raise RuntimeError("cancellation custody raw-write trace target changed")
        target.write_bytes(source.replace(TRACE_NEEDLE, TRACE_INSERTION, 1))
        (target.parent / "native_v1_custody_observer.go").write_bytes(TRACE_HELPER)
        command = tree / "cmd/native_cancel_custody_reference"
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
            ["go", "run", "-mod=readonly", "./cmd/native_cancel_custody_reference"],
            cwd=tree,
        )


def validate(label: str, data: bytes) -> dict:
    document = json.loads(data)
    if document["schema"] != 1:
        raise RuntimeError(f"{label}: unsupported schema")
    if document["selectors"] != {"cancel_v1": "399ff554", "cancel_v2": "b6e1e329"}:
        raise RuntimeError(f"{label}: cancellation selectors changed")
    if document["action_gas"] != {"cancel_v1": 60_000, "cancel_v2": 60_000}:
        raise RuntimeError(f"{label}: cancellation action gas changed")
    if document["v1_object_key"] != "d5e9b51cca65743c66ea6d3b14de88ce61572a6aabff3920ea406943f6749ee9":
        raise RuntimeError(f"{label}: V1 custody object identity changed")
    scenarios = {scenario["name"]: scenario for scenario in document["scenarios"]}
    expected_names = {
        "v1_partial_existing_delegation",
        "v1_full_recreated_delegation",
        "v1_missing_queue",
        "pre_magnolia_v1_missing_validator",
        "v2_non_last_then_last",
    }
    if set(scenarios) != expected_names:
        raise RuntimeError(f"{label}: cancellation scenario set changed")

    successful = {
        "v1_partial_existing_delegation": ("cancel_v1",),
        "v1_full_recreated_delegation": ("cancel_v1",),
        "v2_non_last_then_last": ("cancel_v2_id_1", "cancel_v2_id_2"),
    }
    expected_success = {
        ("v1_partial_existing_delegation", "cancel_v1"): (
            81_464,
            "fc25f8a919d19f2c2dfce21115718abc9ef2b1e0c9218a488f614c75be4184b7",
            9,
            4,
        ),
        ("v1_full_recreated_delegation", "cancel_v1"): (
            81_464,
            "fc25f8a919d19f2c2dfce21115718abc9ef2b1e0c9218a488f614c75be4184b7",
            11,
            4,
        ),
        ("v2_non_last_then_last", "cancel_v2_id_1"): (
            81_656,
            "e0474558d9b6ee7a45f2d6d12effd21909b53360eb73eda6cf0f197031738fee",
            11,
            6,
        ),
        ("v2_non_last_then_last", "cancel_v2_id_2"): (
            81_656,
            "e0474558d9b6ee7a45f2d6d12effd21909b53360eb73eda6cf0f197031738fee",
            12,
            7,
        ),
    }
    expected_queue_prefixes = {
        ("v1_partial_existing_delegation", "cancel_v1"): [
            ("d5e9b51cca65743c66ea6d3b14de88ce61572a6aabff3920ea406943f6749ee9", ""),
            ("0457651292671ad645d4f03ca2fb3583a9f53078f3d5e4518866a2e05dd787b4", ""),
            ("1e9c3648dd8b43db795c3f2a7cfd5233b9685286284c97694d9eef8a158718c0", ""),
            ("0700a38280c043ad1117478c0f798c0a070737b0f630657c201d9494170c65ea", "00000000"),
        ],
        ("v1_full_recreated_delegation", "cancel_v1"): [
            ("d5e9b51cca65743c66ea6d3b14de88ce61572a6aabff3920ea406943f6749ee9", ""),
            ("0457651292671ad645d4f03ca2fb3583a9f53078f3d5e4518866a2e05dd787b4", ""),
            ("1e9c3648dd8b43db795c3f2a7cfd5233b9685286284c97694d9eef8a158718c0", ""),
            ("0700a38280c043ad1117478c0f798c0a070737b0f630657c201d9494170c65ea", "00000000"),
        ],
        ("v2_non_last_then_last", "cancel_v2_id_1"): [
            ("85a91699519be501238684099ed9ff4cbb128caa6bc620d0dcf5b613253cb81d", ""),
            ("5a2487c9a5187232179877b5438d59bc0d2a8eaff0cdb75d4f2c1ab06803281c", "01000000"),
            ("859e6064e61cc17be8795f504c670a033ac038baebf014a1b7c2dc835287a486", "0200000000000000"),
            ("6d991f75b9374f664ae10715702b40dd2464dd020a19c5a32604ada0d4131441", ""),
            ("3e4df70e3c0a7d2a7331cb0960150f71ed26bd2719bfe012aad450ef2c356269", ""),
            ("78b7e952e39d744870c55682f2ff10883238b81ae196b9c0879a79b614fe1d0e", "01000000"),
        ],
        ("v2_non_last_then_last", "cancel_v2_id_2"): [
            ("282013507652eb9502a76b0b23f2b1bef0a5624f4f361a5fd478513330ca6f84", ""),
            ("859e6064e61cc17be8795f504c670a033ac038baebf014a1b7c2dc835287a486", ""),
            ("5a2487c9a5187232179877b5438d59bc0d2a8eaff0cdb75d4f2c1ab06803281c", ""),
            ("78b7e952e39d744870c55682f2ff10883238b81ae196b9c0879a79b614fe1d0e", "00000000"),
            ("e4aac80e69e05ad79a704835e42ba18589a09f024dc238d5d0a2394b085498d5", ""),
            ("56ddf292e134aa3c079c920b59e0be0b6fc431a7b3899537e564cf298f851493", ""),
            ("cd71153d765e23e3e7a1db1fc562517cf29627d2848f1b6bd586bae39f58e39d", "00000000"),
        ],
    }
    for scenario_name, transaction_names in successful.items():
        transactions = {
            transaction["name"]: transaction
            for transaction in scenarios[scenario_name]["transactions"]
        }
        for transaction_name in transaction_names:
            transaction = transactions[transaction_name]
            if (
                transaction["consensus_error"]
                or transaction["execution_error"]
                or transaction["output"]
                or len(transaction["logs"]) != 1
                or not transaction["ordered_raw_writes"]
            ):
                raise RuntimeError(
                    f"{label}/{scenario_name}/{transaction_name}: cancellation behavior changed"
                )
            gas_used, event_topic, write_count, queue_write_count = expected_success[
                (scenario_name, transaction_name)
            ]
            writes = transaction["ordered_raw_writes"]
            if (
                transaction["gas_used"] != gas_used
                or transaction["logs"][0]["topics"][0] != event_topic
                or len(writes) != write_count
                or any(write["address"] != "00" * 19 + "fe" for write in writes)
                or any(write["value"] for write in writes[:1])
            ):
                raise RuntimeError(
                    f"{label}/{scenario_name}/{transaction_name}: gas, event, or raw trace changed"
                )
            # Go removes the custody object and iterable rows before restoring
            # delegation/validator/reward rows. The boundary is four writes for
            # V1, six for a non-last V2 ID, and seven for the last V2 ID.
            if [
                (write["key"], write["value"])
                for write in writes[:queue_write_count]
            ] != expected_queue_prefixes[(scenario_name, transaction_name)]:
                raise RuntimeError(
                    f"{label}/{scenario_name}/{transaction_name}: queue prefix changed"
                )

    failures = {
        ("v1_missing_queue", "cancel_v1"): "Undelegation does not exist",
        ("pre_magnolia_v1_missing_validator", "cancel_v1"): "Validator does not exist",
        ("v2_non_last_then_last", "cancel_v2_missing"): "Undelegation does not exist",
    }
    for (scenario_name, transaction_name), expected_error in failures.items():
        transaction = next(
            transaction
            for transaction in scenarios[scenario_name]["transactions"]
            if transaction["name"] == transaction_name
        )
        if (
            transaction["consensus_error"]
            or transaction["execution_error"] != expected_error
            or transaction["output"]
            or transaction["logs"]
            or transaction["ordered_raw_writes"]
        ):
            raise RuntimeError(
                f"{label}/{scenario_name}/{transaction_name}: failure boundary changed"
            )

    expected_before = {
        "period": 0,
        "delegator_balance": "1000",
        "contract_balance": "1000",
        "validator_stake": "1000",
        "total_delegated": "1000",
        "validator_exists": True,
    }
    for scenario in scenarios.values():
        if scenario["before"] != expected_before:
            raise RuntimeError(f"{label}/{scenario['name']}: seed state changed")
    for name in (
        "v1_partial_existing_delegation",
        "v1_full_recreated_delegation",
        "v2_non_last_then_last",
    ):
        if scenarios[name]["after_block"] != {**expected_before, "period": 1}:
            raise RuntimeError(f"{label}/{name}: cancellation did not restore principal")
    if scenarios["pre_magnolia_v1_missing_validator"]["after_block"] != {
        **expected_before,
        "period": 1,
        "validator_stake": "0",
        "total_delegated": "0",
        "validator_exists": False,
    }:
        raise RuntimeError(f"{label}: pre-Magnolia missing-validator witness changed")
    v2_transactions = {
        transaction["name"]: transaction
        for transaction in scenarios["v2_non_last_then_last"]["transactions"]
    }
    for transaction_name, expected_id in (
        ("undelegate_v2_id_1", 1),
        ("undelegate_v2_id_2", 2),
    ):
        if int(v2_transactions[transaction_name]["output"], 16) != expected_id:
            raise RuntimeError(f"{label}/{transaction_name}: V2 ID allocation changed")
    return document


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    for label, data in artifacts.items():
        validate(label, data)
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("pinned cancellation custody references diverged")
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"], text=True).strip(),
        "scope": (
            "Actual StateTransition V1/V2 cancelUndelegate custody, ordered raw-write "
            "instrumentation, existing/recreated delegation and V2 non-last/last queue removal; "
            "pre-Magnolia missing-validator observation is reference-only and no production routing is claimed"
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
                raise RuntimeError(f"cancellation custody manifest mismatch: {field}")
        for label, data in artifacts.items():
            if data != (FIXTURES / f"{label}.json").read_bytes():
                raise RuntimeError(f"cancellation custody fixture mismatch: {label}")
    print("Both pinned cancellation custody references " + ("recorded" if args.record else "reproduced"))
    print("Public/local byte-identical:", manifest["public_local_byte_identical"])


if __name__ == "__main__":
    main()
