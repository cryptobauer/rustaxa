#!/usr/bin/env python3
"""Measure/reproduce literal H1 TraceRunner over actual captured canonical P0."""
import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT
import native_reward_claim_all_underfunded_reference as boundary
import native_reward_claim_all_trace_reference as trace

HERE = Path(__file__).resolve().parent
SOURCE = HERE / "native_reward_claim_all_h1_trace_reference.go"
SUPPORT = [*boundary.SUPPORT, HERE / "native_redelegate_trace_reference.go"]
FIXTURES = HERE / "fixtures/native_reward_claim_all_h1_trace"
DB_SOURCE = HERE / "native_simulation_reference.go"
DB_SHA = "e5bb91c092024b2891ac6217369584588c0798de358d017323b2726016cfff60"
SEED_SHA = "02fa9f74a2f5a7f5185d69a4c8f46f95f32c364feb248cc854046cbbb282137c"
COMMIT_NEEDLE = b"\tdatabase.pending = nil\n\treturn nil\n"
READER_NEEDLE = b"\treturn nativeSimulationHistorical{latest: database, period: block}\n"
TARGET_HELPER = b'''package state_dry_runner
import ("github.com/Taraxa-project/taraxa-evm/core/vm";"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm")
var claimAllH1TraceTargetObserver func(vm.ExecutionResult,error,*state_evm.BlockState,*vm.Transaction)
func SetClaimAllH1TraceTargetObserver(observer func(vm.ExecutionResult,error,*state_evm.BlockState,*vm.Transaction)){claimAllH1TraceTargetObserver=observer}
func observeClaimAllH1TraceTarget(ret vm.ExecutionResult,err error,state *state_evm.BlockState,trx *vm.Transaction){if claimAllH1TraceTargetObserver!=nil{claimAllH1TraceTargetObserver(ret,err,state,trx)}}
'''
ACCOUNT_HELPER = b'''package state_evm
import "github.com/Taraxa-project/taraxa-evm/common"
var claimAllH1AccountReadObserver func(string,*common.Address)
func SetClaimAllH1AccountReadObserver(observer func(string,*common.Address)){claimAllH1AccountReadObserver=observer}
func observeClaimAllH1AccountRead(kind string,address *common.Address){if claimAllH1AccountReadObserver!=nil{claimAllH1AccountReadObserver(kind,address)}}
'''


def digest(data):
    return hashlib.sha256(data).hexdigest()


def value_digest(value):
    return digest(json.dumps(value, sort_keys=True, separators=(",", ":")).encode())


def run_reference(label, control, scratch):
    with tempfile.TemporaryDirectory(prefix=label + "-", dir=scratch) as directory:
        tree = Path(directory)
        archive = subprocess.check_output(["rtk", "proxy", "git", "-C", str(ROOT / "submodules/taraxa-evm"), "archive", REVISIONS[label]])
        subprocess.run(["rtk", "proxy", "tar", "-x", "-C", directory], input=archive, check=True)
        patches = []
        instrumented = {}

        def change(path, expected, transform):
            target = tree / path
            original = target.read_bytes()
            assert digest(original) == expected, str(path)
            modified = transform(original)
            target.write_bytes(modified)
            patches.extend(difflib.unified_diff(original.decode().splitlines(True), modified.decode().splitlines(True), fromfile="a/" + str(path), tofile="b/" + str(path)))
            instrumented[str(path)] = digest(modified)

        once = boundary.replace_once
        custody, reward, h1 = boundary.custody, boundary.reward, boundary.h1

        def account(data):
            for method, kind in [("GetBalance", "balance"), ("GetNonce", "nonce")]:
                needle = ("func (self *Account) " + method + "() *big.Int {\n").encode()
                data = once(data, needle, needle if control else needle + b'\tobserveClaimAllH1AccountRead("' + kind.encode() + b'",&self.addr)\n')
            for operation, expression in [("add", "Add"), ("sub", "Sub")]:
                needle = ("self.set_balance(new(big.Int)." + expression + "(self.Balance, amount))").encode()
                insertion = needle if control else needle + b'\n\t\tobserveRewardBalance(self.addr,"' + operation.encode() + b'",amount,self.Balance)'
                data = once(data, needle, insertion)
            return once(data, custody.TRACE_NEEDLE, custody.TRACE_NEEDLE if control else custody.TRACE_INSERTION)

        change(custody.TRACE_TARGET, custody.TRACE_SHA256, account)
        (tree / custody.TRACE_TARGET).parent.joinpath("native_v1_custody_observer.go").write_bytes(custody.TRACE_HELPER)
        (tree / custody.TRACE_TARGET).parent.joinpath("reward_balance_observer.go").write_bytes(reward.BALANCE_HELPER)
        (tree / custody.TRACE_TARGET).parent.joinpath("claim_all_h1_account_read_observer.go").write_bytes(ACCOUNT_HELPER)
        change(h1.VM_TARGET, h1.VM_SHA[label], lambda data: once(once(data, h1.VM_NEEDLE, b"claimAllBoundaryRequired(precompiled, frame, self)"), boundary.RUN_NEEDLE, b"claimAllBoundaryRun(precompiled, frame, self)"))
        (tree / h1.VM_TARGET).parent.joinpath("claim_all_boundary_observer.go").write_bytes(boundary.VM_HELPER)
        (tree / h1.VM_TARGET).parent.joinpath("reward_h1_quote_observer.go").write_bytes(h1.QUOTE_HELPER)
        change(boundary.STORAGE_TARGET, boundary.STORAGE_SHA, lambda data: once(once(data, boundary.LOGICAL_NEEDLE, boundary.LOGICAL_NEEDLE if control else boundary.LOGICAL_NEEDLE + b'\tobserveClaimAllBoundaryRead("logical",self.contract_address,k)\n'), boundary.BACKEND_NEEDLE, boundary.BACKEND_NEEDLE if control else b'\tobserveClaimAllBoundaryRead("backend",self.contract_address,k)\n' + boundary.BACKEND_NEEDLE))
        (tree / boundary.STORAGE_TARGET).parent.joinpath("claim_all_boundary_read_observer.go").write_bytes(boundary.READ_HELPER)
        change(trace.TRACE_TARGET, trace.TRACE_SHA, lambda data: once(data, trace.TRACE_NEEDLE, b"\t\tret, targetError := evm.Main(&trx)\n\t\tobserveClaimAllH1TraceTarget(ret,targetError,block_state,&trx)\n"))
        (tree / trace.TRACE_TARGET).parent.joinpath("claim_all_h1_trace_observer.go").write_bytes(TARGET_HELPER)
        # Declarations used by the accepted support's unused H1 entrypoint.
        assert digest((tree / h1.DRY_TARGET).read_bytes()) == h1.DRY_SHA
        (tree / h1.DRY_TARGET).parent.joinpath("reward_h1_post_observer.go").write_bytes(h1.POST_HELPER)
        command = tree / "cmd/claim_all_h1_trace"
        command.mkdir(parents=True)
        for index, path in enumerate(SUPPORT):
            original = path.read_bytes()
            if path == DB_SOURCE:
                assert digest(original) == DB_SHA
                modified = once(original, COMMIT_NEEDLE, b"\tdatabase.pending = nil\n\tobserveH1TraceCommit(database)\n\treturn nil\n")
                modified = once(modified, READER_NEEDLE, b"\tobserveH1TraceReader(block)\n" + READER_NEEDLE)
                patches.extend(difflib.unified_diff(original.decode().splitlines(True), modified.decode().splitlines(True), fromfile="a/fixture/" + path.name, tofile="b/fixture/" + path.name))
            else:
                modified = original
            modified = once(modified, b"func main() {", f"func unusedH1TraceSupport{index}Main() {{".encode())
            (command / path.name).write_bytes(modified)
            instrumented["fixture/" + path.name] = digest(modified)
        assert digest((HERE / "native_redelegate_reward_h1_reference.go").read_bytes()) == SEED_SHA
        (command / SOURCE.name).write_bytes(SOURCE.read_bytes())
        result = subprocess.run(["rtk", "proxy", "go", "run", "-mod=readonly", "./cmd/claim_all_h1_trace"], cwd=tree, capture_output=True, env={**os.environ, "CLAIM_ALL_H1_TRACE_CONTROL": "1" if control else "0"})
        if result.returncode:
            raise RuntimeError(result.stdout.decode() + result.stderr.decode())
        lines = result.stdout.splitlines(keepends=True)
        assert lines, "Missing actual trace output"
        return (lines[-1], b"".join(lines[:-1]), result.stderr), {"instrumented_sha256": instrumented, "patch": "".join(patches)}


def validate(document, control=False):
    accepted = json.loads((HERE / "fixtures/native_redelegate_reward_h1/public.json").read_text())
    assert document["schema"] == 1 and document["p0_capture_count"] == 1
    assert document["repeat_identical"] and document["source_unchanged_after_each"]
    assert document["p1_before"] == document["p1_after"] == accepted["state_before"]
    assert len(document["p1_before"]["seed_rows"]) == 92
    assert document["p0_source_before"] == document["p0_source_after"]
    p0 = document["p0_source_before"]
    assert p0["period"] == 0 and p0["root"] != document["p1_before"]["root"]
    assert len(p0["root"]) == 64 and p0["seed_rows"]
    assert sum(p0["column_counts"]) == len(p0["seed_rows"])
    # Fixed from the first actual four-run capture; not source predictions.
    assert p0["root"] == "58a5dd667bda27fcf89ee0739bb030f469ff2317b3a3a0035dcbee9a046b813f"
    assert p0["column_counts"] == [14, 7, 17, 11, 33]
    assert len(p0["seed_rows"]) == 82
    assert value_digest(p0["seed_rows"]) == "bbf8adfde7963b6004cbdf568713a485796a4b044fb9f5f1cb2f8280d5b9206e"
    assert value_digest(p0["view"]) == "7fb004d7df785bb2ea23fb22dfd6f85a83e650d07c78c87c74444f9546cdaa4b"
    assert len(document["cases"]) == 1
    case = document["cases"][0]
    assert case["name"] == "claim_all_H1_P0_empty_prefix" and not case["prefix"]
    assert case["reader_periods"] == [0] and case["execution_period"] == 1
    assert len(case["targets"]) == len(case["result"]) == 1
    target = case["targets"][0]
    assert target == {"sender": document["caller"], "to": "00" * 19 + "fe", "nonce": str(2**512), "gas_price": "0", "gas": 200000, "value": "0", "input": "0b83a727"}
    assert case["post"]["supplied_nonce"] == target["nonce"]
    assert case["diagnostics"] == ""
    assert case["result"] == [{"gas": 111272, "failed": False, "returnValue": "", "structLogs": []}]
    assert case["quotes"] == [{"required_gas": 90000, "supplied_native_gas": 178728, "caller": document["caller"], "input": "0b83a727", "depth": 0, "value": "0"}]
    assert case["runs"] == [{"supplied_native_gas": 178728, "output": "", "error": ""}]
    post = case["post"]
    assert post["gas_used"] == 111272
    assert post["consensus_error"] == post["execution_error"] == post["parent_error"] == post["return"] == ""
    assert post["logs"] == []
    assert post["accounts"] == {
        document["caller"]: {"balance": "3000", "nonce": str(2**512 + 1)},
        document["owner"]: {"balance": "1000", "nonce": "0"},
        target["to"]: {"balance": "4000", "nonce": "1"},
    }
    assert value_digest(case["prior_raw"]) == "134a4dfd6ebc60f40fc7c0184ad8fe70831900e19a6b3113ccf45eca3729cb69"
    assert value_digest(post["after_raw"]) == "682a609f2d26c068774d87021c4f3a3975057eb8722c11fa1323ebf141949a2f"
    assert case["ordered_balance_effects"] == []
    assert case["ordered_account_scalar_reads"] == []
    if control:
        assert case["ordered_reads"] == case["ordered_raw_writes"] == []
    else:
        assert len(case["ordered_reads"]) == 48 and len(case["ordered_raw_writes"]) == 12
        assert value_digest(case["ordered_reads"]) == "2bd3a034a7f40f230b2b56b5e6002d24d0be423bd5d3c3184b7d76851ec30c4b"
        assert value_digest(case["ordered_raw_writes"]) == "d456677ef2f99ee81fdc6b38f2ac44c2df17a665c16048a38236a02596604a33"
        expected = dict(case["prior_raw"])
        for write in case["ordered_raw_writes"]:
            assert write["address"] == target["to"] and write["key"] in expected
            expected[write["key"]] = write["value"]
        assert expected == post["after_raw"]


def strip(value):
    if isinstance(value, dict):
        return {key: strip(item) for key, item in value.items() if key not in boundary.OBSERVATIONS | {"ordered_account_scalar_reads"}}
    if isinstance(value, list):
        return [strip(item) for item in value]
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    parser.add_argument("--capture", type=Path, help="Save first measurements before validation")
    args = parser.parse_args()
    scratch = Path(os.environ.get("RUSTAXA_H1_TRACE_SCRATCH", "/home/fry/artifacts/claim-all-h1-trace-producer-scratch"))
    scratch.mkdir(parents=True, exist_ok=True)
    outputs, details = {}, {}
    for control in [False, True]:
        for label in REVISIONS:
            name = label + (".control" if control else "")
            outputs[name], details[name] = run_reference(label, control, scratch)
            if args.capture:
                args.capture.mkdir(parents=True, exist_ok=True)
                for suffix, value in zip(["json", "stdout.txt", "stderr.txt"], outputs[name], strict=True):
                    args.capture.joinpath(name + "." + suffix).write_bytes(value)
                args.capture.joinpath(name + ".instrumentation.json").write_text(json.dumps(details[name], indent=2, sort_keys=True) + "\n")
    assert outputs["public"] == outputs["local"]
    assert outputs["public.control"] == outputs["local.control"]
    for label in REVISIONS:
        observed, control = json.loads(outputs[label][0]), json.loads(outputs[label + ".control"][0])
        validate(observed)
        validate(control, True)
        assert strip(observed) == strip(control)
    files = {}
    for label, output in outputs.items():
        for suffix, value in zip(["json", "stdout.txt", "stderr.txt"], output, strict=True):
            files[label + "." + suffix] = value
        # JSON keeps exact diff whitespace without adding trailing whitespace
        # to a tracked patch document. The patch string is unchanged.
        files[label + ".instrumentation.json"] = (json.dumps(details[label], indent=2, sort_keys=True) + "\n").encode()
    manifest = {
        "schema": 1, "references": REVISIONS, "profile": "literal_H1_TraceRunner_actual_canonical_P0_empty_prefix",
        "source_sha256": {path.name: digest(path.read_bytes()) for path in [SOURCE, Path(__file__), *SUPPORT]},
        "source_harness_sha256": {path.name: digest(path.read_bytes()) for path in [Path(boundary.__file__), Path(trace.__file__), Path(boundary.h1.__file__), Path(boundary.custody.__file__), Path(boundary.reward.__file__)]},
        "accepted_P1_sha256": digest((HERE / "fixtures/native_redelegate_reward_h1/public.json").read_bytes()),
        "copy_targets": {str(trace.TRACE_TARGET): trace.TRACE_SHA, str(boundary.STORAGE_TARGET): boundary.STORAGE_SHA, str(boundary.h1.VM_TARGET): boundary.h1.VM_SHA, str(boundary.h1.DRY_TARGET): boundary.h1.DRY_SHA, str(boundary.custody.TRACE_TARGET): boundary.custody.TRACE_SHA256},
        "copy_helpers": {"target": digest(TARGET_HELPER), "VM": digest(boundary.VM_HELPER), "read": digest(boundary.READ_HELPER), "account_scalar_read": digest(ACCOUNT_HELPER), "raw": digest(boundary.custody.TRACE_HELPER), "balance": digest(boundary.reward.BALANCE_HELPER), "quote_declaration": digest(boundary.h1.QUOTE_HELPER), "post_declaration": digest(boundary.h1.POST_HELPER)},
        "instrumented_sha256": {name: detail["instrumented_sha256"] for name, detail in details.items()},
        "outputs": {name: digest(value) for name, value in files.items()},
        "P0_capture_and_quote_run_target_copies_in_controls": True,
        "control_execution_identical": True,
        "go_version": subprocess.check_output(["rtk", "proxy", "go", "version"], text=True).strip(),
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
    if args.record or not args.capture:
        for name, value in files.items():
            path = FIXTURES / name
            if args.record:
                path.write_bytes(value)
            else:
                assert path.read_bytes() == value, str(path)
    print("Both actual H1/P0 trace pins and controls matched; captured P0 and canonical P1 remained unchanged")


if __name__ == "__main__":
    main()
