#!/usr/bin/env python3
"""Reproduce the bounded Aspen2 reward phase at both pinned Go revisions."""

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile

from reference import REVISIONS

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = pathlib.Path(
    os.environ.get("TARAXA_EVM_SOURCE", "/workspaces/rustaxa-evm/submodules/taraxa-evm")
)
SOURCES = (
    "mixed_period_reference.go",
    "mixed_period_reference_public.go",
    "current_rewards_reference.go",
)
TRACE_TARGET = pathlib.Path("taraxa/state/state_evm/account.go")
TRACE_SHA256 = "070a30bd1f0d8a34ca4e0037346f181a946944f8e38a25b32ec91e68d7577568"
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
MAIN_NEEDLE = b'''func main() {
	mode := flag.String("mode", "batched", "batched or observer")
	scenario := flag.String("scenario", "initial", "initial or full")
	flag.Parse()
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if *scenario == "full" {
		must(encoder.Encode(runFullWitness(*mode)))
		return
	}
	if *scenario != "initial" {
		panic("scenario must be initial or full")
	}
	must(encoder.Encode(runWitness(*mode)))
}
'''
MAIN_REPLACEMENT = b'''func main() {
	_ = flag.CommandLine
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(runCurrentRewardsWitness()))
}
'''


def patch_archive(tree: pathlib.Path) -> None:
    target = tree / TRACE_TARGET
    source = target.read_bytes()
    if hashlib.sha256(source).hexdigest() != TRACE_SHA256:
        raise RuntimeError("reward raw-write trace target hash differs")
    if source.count(TRACE_NEEDLE) != 1:
        raise RuntimeError("reward raw-write trace insertion point is not unique")
    target.write_bytes(source.replace(TRACE_NEEDLE, TRACE_INSERTION, 1))
    (target.parent / "current_rewards_raw_write_observer.go").write_bytes(TRACE_HELPER)
    command_main = tree / "cmd/current_rewards_reference/mixed_period_reference.go"
    source = command_main.read_bytes()
    if source.count(MAIN_NEEDLE) != 1:
        raise RuntimeError("mixed-period main function differs")
    command_main.write_bytes(source.replace(MAIN_NEEDLE, MAIN_REPLACEMENT, 1))


def run_reference(revision: str) -> bytes:
    with tempfile.TemporaryDirectory(prefix="rustaxa-current-rewards-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        command = tree / "cmd/current_rewards_reference"
        command.mkdir(parents=True)
        for name in SOURCES:
            (command / name).write_bytes((HERE / name).read_bytes())
        patch_archive(tree)
        return subprocess.check_output(
            ["go", "run", "-mod=readonly", "./cmd/current_rewards_reference"], cwd=tree
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"]).decode().strip(),
        "scope": (
            "Synthetic Aspen2 activation with two ordered singleton-validator distributions; actual "
            "DistributeRewards/EndBlockCall and unnormalized raw-write trace; no multi-entry Go map order, "
            "snapshot, jailed cleanup, redelegation fix, physical-retention, persistence, or production claim"
        ),
        "public_local_identical": artifacts["public"] == artifacts["local"],
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
        "exporter_sha256": hashlib.sha256((HERE / "current_rewards_reference.go").read_bytes()).hexdigest(),
        "trace_patch": {"target": str(TRACE_TARGET), "sha256": TRACE_SHA256},
    }
    fixtures = HERE / "fixtures"
    if args.record:
        for label, data in artifacts.items():
            (fixtures / f"current_rewards_{label}.json").write_bytes(data)
        (fixtures / "current_rewards_manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        saved = json.loads((fixtures / "current_rewards_manifest.json").read_text())
        for field in (
            "schema", "references", "scope", "public_local_identical", "sha256",
            "exporter_sha256", "trace_patch",
        ):
            if saved[field] != manifest[field]:
                raise RuntimeError(f"reward manifest mismatch: {field}")
        for label, data in artifacts.items():
            if data != (fixtures / f"current_rewards_{label}.json").read_bytes():
                raise RuntimeError(f"reward fixture mismatch: {label}")
    print("Both pinned Aspen2 reward references " + ("recorded" if args.record else "reproduced"))
    print("Public/local identical:", manifest["public_local_identical"])


if __name__ == "__main__":
    main()
