#!/usr/bin/env python3
"""Reproduce pinned V1 undelegation custody fixtures; use --record to update."""

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
GO_SOURCE = HERE / "native_v1_custody_reference.go"
SUPPORT_SOURCE = HERE / "native_simulation_reference.go"
FIXTURES = HERE / "fixtures/native_v1_custody"
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
    with tempfile.TemporaryDirectory(prefix="rustaxa-native-v1-custody-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        target = tree / TRACE_TARGET
        source = target.read_bytes()
        if hashlib.sha256(source).hexdigest() != TRACE_SHA256 or source.count(TRACE_NEEDLE) != 1:
            raise RuntimeError("V1 custody raw-write trace target changed")
        target.write_bytes(source.replace(TRACE_NEEDLE, TRACE_INSERTION, 1))
        (target.parent / "native_v1_custody_observer.go").write_bytes(TRACE_HELPER)
        command = tree / "cmd/native_v1_custody_reference"
        command.mkdir(parents=True)
        support = SUPPORT_SOURCE.read_text().replace(
            "func main() {", "func nativeSimulationSupportMain() {", 1
        )
        (command / "native_simulation_support.go").write_text(support)
        (command / GO_SOURCE.name).write_bytes(GO_SOURCE.read_bytes())
        return subprocess.check_output(
            ["go", "run", "-mod=readonly", "./cmd/native_v1_custody_reference"], cwd=tree
        )


def validate(label: str, data: bytes) -> dict:
    document = json.loads(data)
    if document["selectors"] != {
        "confirm_undelegate": "45a02561",
        "undelegate": "4d99dd16",
    }:
        raise RuntimeError(f"{label}: selectors changed")
    if document["action_gas"] != {"confirm_undelegate": 20_000, "undelegate": 60_000}:
        raise RuntimeError(f"{label}: native action gas changed")
    scenarios = {scenario["name"]: scenario for scenario in document["scenarios"]}
    expected_names = {
        "magnolia_ficus_partial_cornus_lock",
        "magnolia_ficus_terminal",
        "magnolia_pre_ficus_partial",
        "pre_magnolia_terminal",
        "cacti_lock_priority",
    }
    if set(scenarios) != expected_names:
        raise RuntimeError(f"{label}: scenario set changed")
    dpos_address = "00000000000000000000000000000000000000fe"
    undelegated_topic = "4d10bd049775c77bd7f255195afba5088028ecb3c7c277d393ccff7934f2f92c"
    confirmed_topic = "f8bef3a6fe3b4c932b5b51c6472a89f171d039f4bacf18cff632208938bf0426"
    delegator_topic = "00" * 31 + "d1"
    validator_topic = "00" * 31 + "31"
    iterable_item_key = "0457651292671ad645d4f03ca2fb3583a9f53078f3d5e4518866a2e05dd787b4"
    iterable_position_key = "1e9c3648dd8b43db795c3f2a7cfd5233b9685286284c97694d9eef8a158718c0"
    iterable_count_key = "0700a38280c043ad1117478c0f798c0a070737b0f630657c201d9494170c65ea"
    validator_key = "4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a"
    expected_before = {
        "period": 0, "delegator_balance": "1000", "contract_balance": "1000",
        "validator_stake": "1000", "total_delegated": "1000", "validator_exists": True,
    }
    for scenario in scenarios.values():
        transactions = {transaction["name"]: transaction for transaction in scenario["transactions"]}
        nonces = [transaction["nonce"] for transaction in scenario["transactions"]]
        if nonces != ["0", "1", "2", "3", "4"]:
            raise RuntimeError(f"{label}/{scenario['name']}: nonce sequence changed")
        if (
            transactions["undelegate"]["gas_used"] != 81_720
            or transactions["duplicate_v1"]["gas_used"] != 81_656
        ):
            raise RuntimeError(f"{label}/{scenario['name']}: undelegate gas changed")
        confirm_names = ("early_confirm", "mature_confirm", "missing_confirm")
        if any(transactions[name]["gas_used"] != 41_464 for name in confirm_names):
            raise RuntimeError(f"{label}/{scenario['name']}: confirmation gas changed")
        if transactions["undelegate"]["execution_error"] or not transactions["undelegate"]["ordered_raw_writes"]:
            raise RuntimeError(f"{label}/{scenario['name']}: undelegate failed")
        if transactions["duplicate_v1"]["execution_error"] != "Undelegation already exist":
            raise RuntimeError(f"{label}/{scenario['name']}: duplicate error changed")
        if transactions["early_confirm"]["execution_error"] != "Undelegation is not yet ready to be withdrawn":
            raise RuntimeError(f"{label}/{scenario['name']}: early error changed")
        if transactions["mature_confirm"]["execution_error"] or not transactions["mature_confirm"]["ordered_raw_writes"]:
            raise RuntimeError(f"{label}/{scenario['name']}: mature confirmation failed")
        if transactions["missing_confirm"]["execution_error"] != "Undelegation does not exist":
            raise RuntimeError(f"{label}/{scenario['name']}: missing error changed")
        for failure in ("duplicate_v1", "early_confirm", "missing_confirm"):
            if transactions[failure]["ordered_raw_writes"] or transactions[failure]["logs"]:
                raise RuntimeError(f"{label}/{scenario['name']}: failed operation emitted effects")
        for transaction in transactions.values():
            if transaction["consensus_error"] or transaction["output"]:
                raise RuntimeError(f"{label}/{scenario['name']}: consensus/output behavior changed")
        if scenario["cacti"] and scenario["unlock_period"] != 8:
            raise RuntimeError(f"{label}: Cacti lock did not take priority")
        if not scenario["cacti"] and scenario["unlock_period"] != 4:
            raise RuntimeError(f"{label}: Cornus lock changed")
        amount = int(scenario["amount"])
        amount_word = amount.to_bytes(32, "big").hex()
        expected_rlp = {
            (300, 4): "c482012c04",
            (1000, 4): "c48203e804",
            (300, 8): "c482012c08",
        }[(amount, scenario["unlock_period"])]
        undelegate_writes = transactions["undelegate"]["ordered_raw_writes"]
        expected_queue_create = [
            {
                "address": dpos_address,
                "key": scenario["v1_object_key"],
                "value": expected_rlp,
            },
            {"address": dpos_address, "key": iterable_item_key, "value": "00" * 19 + "31"},
            {"address": dpos_address, "key": iterable_position_key, "value": "01000000"},
            {"address": dpos_address, "key": iterable_count_key, "value": "01000000"},
        ]
        if undelegate_writes[-4:] != expected_queue_create:
            raise RuntimeError(f"{label}/{scenario['name']}: V1 RLP/key/create order changed")
        expected_queue_remove = [
            {"address": dpos_address, "key": scenario["v1_object_key"], "value": ""},
            {"address": dpos_address, "key": iterable_item_key, "value": ""},
            {"address": dpos_address, "key": iterable_position_key, "value": ""},
            {"address": dpos_address, "key": iterable_count_key, "value": "00000000"},
        ]
        mature_writes = transactions["mature_confirm"]["ordered_raw_writes"]
        if mature_writes[:4] != expected_queue_remove:
            raise RuntimeError(f"{label}/{scenario['name']}: V1 queue removal order changed")
        expected_topics = [delegator_topic, validator_topic]
        successful_events = (
            ("undelegate", undelegated_topic),
            ("mature_confirm", confirmed_topic),
        )
        for name, event_topic in successful_events:
            logs = transactions[name]["logs"]
            if len(logs) != 1 or logs[0] != {
                "address": dpos_address, "topics": [event_topic, *expected_topics], "data": amount_word,
            }:
                raise RuntimeError(f"{label}/{scenario['name']}: {name} log changed")
        if scenario["before"] != expected_before:
            raise RuntimeError(f"{label}/{scenario['name']}: seeded custody state changed")
        remaining = 1000 - amount
        expected_after_undelegate = {
            "period": 1, "delegator_balance": "1000", "contract_balance": "1000",
            "validator_stake": str(remaining), "total_delegated": str(remaining),
            "validator_exists": amount != 1000 or scenario["magnolia"],
        }
        expected_after_confirm = {
            "period": scenario["unlock_period"], "delegator_balance": str(1000 + amount),
            "contract_balance": str(1000 - amount), "validator_stake": str(remaining),
            "total_delegated": str(remaining), "validator_exists": amount != 1000,
        }
        if (
            scenario["after_undelegate_block"] != expected_after_undelegate
            or scenario["after_confirm_block"] != expected_after_confirm
        ):
            raise RuntimeError(f"{label}/{scenario['name']}: custody balance/stake transition changed")

    ficus_partial = scenarios["magnolia_ficus_partial_cornus_lock"]
    pre_ficus_partial = scenarios["magnolia_pre_ficus_partial"]
    ficus_transactions = {
        transaction["name"]: transaction for transaction in ficus_partial["transactions"]
    }
    pre_ficus_transactions = {
        transaction["name"]: transaction
        for transaction in pre_ficus_partial["transactions"]
    }
    ficus_mature = ficus_transactions["mature_confirm"]
    pre_ficus_mature = pre_ficus_transactions["mature_confirm"]
    if (
        len(ficus_mature["ordered_raw_writes"]) != 5
        or ficus_mature["ordered_raw_writes"][4]["key"] != validator_key
    ):
        raise RuntimeError(f"{label}: Ficus validator-count rewrite changed")
    if len(pre_ficus_mature["ordered_raw_writes"]) != 4:
        raise RuntimeError(f"{label}: pre-Ficus confirmation unexpectedly rewrote validator count")
    magnolia_terminal = scenarios["magnolia_ficus_terminal"]
    pre_magnolia_terminal = scenarios["pre_magnolia_terminal"]
    magnolia_transactions = {
        transaction["name"]: transaction
        for transaction in magnolia_terminal["transactions"]
    }
    pre_magnolia_transactions = {
        transaction["name"]: transaction
        for transaction in pre_magnolia_terminal["transactions"]
    }
    if (
        len(magnolia_transactions["undelegate"]["ordered_raw_writes"]) != 13
        or len(magnolia_transactions["mature_confirm"]["ordered_raw_writes"]) != 13
    ):
        raise RuntimeError(f"{label}: Magnolia terminal cleanup placement changed")
    if (
        len(pre_magnolia_transactions["undelegate"]["ordered_raw_writes"]) != 19
        or len(pre_magnolia_transactions["mature_confirm"]["ordered_raw_writes"]) != 4
    ):
        raise RuntimeError(f"{label}: pre-Magnolia terminal cleanup placement changed")
    return document


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    for label, data in artifacts.items():
        validate(label, data)
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("pinned V1 custody references diverged")
    manifest = {
        "schema": 1, "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"], text=True).strip(),
        "scope": (
            "Actual StateTransition V1 undelegate/confirm custody, ordered raw-write "
            "instrumentation, Cornus/Cacti locks and Magnolia/Ficus cleanup boundaries; "
            "no production routing claim"
        ),
        "instrumentation": {
            "target": str(TRACE_TARGET),
            "sha256": TRACE_SHA256,
            "observer_sha256": hashlib.sha256(TRACE_HELPER).hexdigest(),
            "insertion": (
                "synchronous copy-only observer before SetStateRawIrreversibly mutation body"
            ),
        },
        "support_sha256": hashlib.sha256(SUPPORT_SOURCE.read_bytes()).hexdigest(),
        "public_local_byte_identical": artifacts["public"] == artifacts["local"],
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "exporter_sha256": hashlib.sha256(GO_SOURCE.read_bytes()).hexdigest(),
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
            "public_local_byte_identical",
            "sha256",
            "exporter_sha256",
        ):
            if saved[field] != manifest[field]:
                raise RuntimeError(f"V1 custody manifest mismatch: {field}")
        for label, data in artifacts.items():
            if data != (FIXTURES / f"{label}.json").read_bytes():
                raise RuntimeError(f"V1 custody fixture mismatch: {label}")
    print("Both pinned V1 custody references " + ("recorded" if args.record else "reproduced"))
    print("Public/local byte-identical:", manifest["public_local_byte_identical"])


if __name__ == "__main__":
    main()
