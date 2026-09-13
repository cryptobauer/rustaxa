#!/usr/bin/env python3
"""Reproduce pinned Go reward-claim fixtures; use --record to update."""

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
GO_SOURCE = HERE / "native_claims_reference.go"
SIMULATION_SUPPORT = HERE / "native_simulation_reference.go"
CUSTODY_SUPPORT = HERE / "native_v1_custody_reference.go"
REWARD_SUPPORT = HERE / "native_cancel_reward_reference.go"
FIXTURES = HERE / "fixtures/native_claims"
TRACE_TARGET = pathlib.Path("taraxa/state/state_evm/account.go")
TRACE_SHA256 = "070a30bd1f0d8a34ca4e0037346f181a946944f8e38a25b32ec91e68d7577568"
TRACE_NEEDLE = b"func (self *Account) SetStateRawIrreversibly(key *common.Hash, value []byte) {\n"
TRACE_INSERTION = TRACE_NEEDLE + b"\tobserveNativeV1CustodyRawWrite(self.addr, *key, value)\n"
TRACE_HELPER = b'''package state_evm

import "github.com/Taraxa-project/taraxa-evm/common"

var nativeV1CustodyRawWriteObserver func(common.Address, common.Hash, []byte)

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
    with tempfile.TemporaryDirectory(prefix="rustaxa-native-claims-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        target = tree / TRACE_TARGET
        source = target.read_bytes()
        if hashlib.sha256(source).hexdigest() != TRACE_SHA256 or source.count(TRACE_NEEDLE) != 1:
            raise RuntimeError("claims raw-write trace target changed")
        target.write_bytes(source.replace(TRACE_NEEDLE, TRACE_INSERTION, 1))
        (target.parent / "native_claims_observer.go").write_bytes(TRACE_HELPER)
        command = tree / "cmd/native_claims_reference"
        command.mkdir(parents=True)
        supports = (
            (SIMULATION_SUPPORT, "func nativeSimulationSupportMain() {"),
            (CUSTODY_SUPPORT, "func nativeV1CustodySupportMain() {"),
            (REWARD_SUPPORT, "func nativeCancelRewardSupportMain() {"),
        )
        for path, replacement in supports:
            text = path.read_text().replace("func main() {", replacement, 1)
            (command / path.name).write_text(text)
        (command / GO_SOURCE.name).write_bytes(GO_SOURCE.read_bytes())
        return subprocess.check_output(
            ["go", "run", "-mod=readonly", "./cmd/native_claims_reference"], cwd=tree
        )


def validate(label: str, data: bytes) -> dict:
    document = json.loads(data)
    if document["schema"] != 1 or document["selectors"] != {
        "claim_rewards": "ef5cfb8c",
        "claim_commission_rewards": "d0eebfe2",
    } or document["action_gas"] != {
        "claim_rewards": 40_000,
        "claim_commission_rewards": 20_000,
    }:
        raise RuntimeError(f"{label}: claims contract changed")
    scenarios = {scenario["name"]: scenario for scenario in document["scenarios"]}
    if set(scenarios) != {"delegator_accrued", "commission_accrued"}:
        raise RuntimeError(f"{label}: claims scenarios changed")
    for name, selector, gas, amounts in (
        ("delegator_accrued", "ef5cfb8c", 40_000, (198, None)),
        ("commission_accrued", "d0eebfe2", 20_000, (2, 0)),
    ):
        scenario = scenarios[name]
        if scenario["reward_minted"] != "200" or len(scenario["transactions"]) != 2:
            raise RuntimeError(f"{label}/{name}: reward lifecycle changed")
        for index, transaction in enumerate(scenario["transactions"]):
            if transaction["selector"] != selector or transaction["consensus_error"] or transaction["execution_error"]:
                raise RuntimeError(f"{label}/{name}: claim failed")
            if transaction["gas_used"] != gas + 21_464:
                raise RuntimeError(f"{label}/{name}: gas changed")
            if amounts[index] is None:
                if transaction["logs"]:
                    raise RuntimeError(f"{label}/{name}: zero delegator claim logged")
            else:
                if len(transaction["logs"]) != 1 or int(transaction["logs"][0]["data"], 16) != amounts[index]:
                    raise RuntimeError(f"{label}/{name}: log amount changed")
    errors = {transaction["name"]: transaction for transaction in document["errors"]}
    expected_errors = {
        "claim_rewards_missing_delegation": "Delegation does not exist",
        "claim_commission_wrong_owner": "This account is not owner of specified validator",
        "claim_commission_missing_validator": "This account is not owner of specified validator",
    }
    if set(errors) != set(expected_errors):
        raise RuntimeError(f"{label}: claim error scenarios changed")
    for name, expected in expected_errors.items():
        transaction = errors[name]
        if transaction["execution_error"] != expected or transaction["consensus_error"] or transaction["logs"] or transaction["ordered_raw_writes"]:
            raise RuntimeError(f"{label}/{name}: error precedence/effects changed")
    return document


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    for label, data in artifacts.items():
        validate(label, data)
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("pinned claims references diverged")
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"], text=True).strip(),
        "scope": "actual live StateTransition reward distribution followed by nonzero and zero-repeat delegator/commission claims; exact errors, logs and raw writes; zero-stake commission and Rust publication/reopen are excluded",
        "instrumentation": {"target": str(TRACE_TARGET), "sha256": TRACE_SHA256, "observer_sha256": hashlib.sha256(TRACE_HELPER).hexdigest(), "insertion": "synchronous copy-only observer before irreversible raw mutation body"},
        "support_sha256": {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in (SIMULATION_SUPPORT, CUSTODY_SUPPORT, REWARD_SUPPORT)},
        "public_local_byte_identical": artifacts["public"] == artifacts["local"],
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "exporter_sha256": hashlib.sha256(GO_SOURCE.read_bytes()).hexdigest(),
        "harness_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
    }
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for label, data in artifacts.items():
            (FIXTURES / f"{label}.json").write_bytes(data)
        (FIXTURES / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    else:
        for label, data in artifacts.items():
            if data != (FIXTURES / f"{label}.json").read_bytes():
                raise RuntimeError(f"{label}: claims fixture is stale")
        recorded = json.loads((FIXTURES / "manifest.json").read_text())
        for key in ("references", "instrumentation", "support_sha256", "sha256", "exporter_sha256"):
            if recorded[key] != manifest[key]:
                raise RuntimeError(f"claims manifest {key} is stale")


if __name__ == "__main__":
    main()
