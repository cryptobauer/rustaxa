#!/usr/bin/env python3
"""Measure P2/P1 TraceRunner gas boundaries before encoding outcome assertions."""
import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
from reference import REVISIONS, ROOT
import native_reward_claim_all_h1_trace_reference as inherited
from native_reward_claim_all_h1_trace_reference import boundary, trace, SUPPORT, DB_SOURCE, DB_SHA, SEED_SHA, COMMIT_NEEDLE, READER_NEEDLE, ACCOUNT_HELPER

HERE = Path(__file__).resolve().parent
SOURCE = HERE / "native_reward_claim_all_underfunded_trace_reference.go"
FIXTURES = HERE / "fixtures/native_reward_claim_all_underfunded_trace"
CAPS = [21271, 21272, 21273, 90000, 111271, 111272, 111273]
TARGET_HELPER = inherited.TARGET_HELPER + b"""
var claimAllBoundaryPrefixObserver func(vm.ExecutionResult,error,*state_evm.BlockState,*vm.Transaction)
func SetClaimAllBoundaryPrefixObserver(observer func(vm.ExecutionResult,error,*state_evm.BlockState,*vm.Transaction)){claimAllBoundaryPrefixObserver=observer}
func observeClaimAllBoundaryPrefix(ret vm.ExecutionResult,err error,state *state_evm.BlockState,trx *vm.Transaction){if claimAllBoundaryPrefixObserver!=nil{claimAllBoundaryPrefixObserver(ret,err,state,trx)}}
"""

def digest(data):
 return hashlib.sha256(data).hexdigest()

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
        change(trace.TRACE_TARGET, trace.TRACE_SHA, lambda data: once(once(data, trace.PREFIX_NEEDLE, b"\t\tprefixRet,prefixError := evm.Main(&trx)\n\t\tobserveClaimAllBoundaryPrefix(prefixRet,prefixError,block_state,&trx)\n"), trace.TRACE_NEEDLE, b"\t\tret, targetError := evm.Main(&trx)\n\t\tobserveClaimAllH1TraceTarget(ret,targetError,block_state,&trx)\n"))
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
    seed = json.loads((HERE / "fixtures/native_redelegate_reward_h1/public.json").read_text())
    boundaries = json.loads((HERE / "fixtures/native_reward_claim_all_underfunded/public.json").read_text())
    funded_trace = json.loads((trace.FIXTURES / "public.json").read_text())
    assert document["schema"] == 1 and document["repeat_identical"] and document["source_unchanged_after_each"]
    assert document["p1_before"] == document["p1_after"] == seed["state_before"]
    assert len(document["p1_before"]["seed_rows"]) == 92
    assert len(document["cases"]) == 14
    for index, case in enumerate(document["cases"]):
        layout, cap = index // 7, CAPS[index % 7]
        base = next(c for c in boundaries["cases"] if c["period"] == 2 and c["gas"] == cap)
        assert case["name"] == f"claim_all_P2_P1_prefix{layout}_gas{cap}"
        assert case["reader_periods"] == [1] and case["execution_period"] == 2
        old = funded_trace["cases"][layout]
        assert case["prefix"] == old["prefix"] and case["prefix_results"] == old["prefix_results"]
        assert case["targets"] == [{**old["targets"][0], "gas": cap}]
        target = case["targets"][0]
        assert target["nonce"] == str(2**512 + layout)
        funded, intrinsic = cap >= 111272, cap == 21271
        expected_gas = 111272 if funded else 21271 if intrinsic else 21272
        assert case["diagnostics"] == ""
        assert case["result"] == [{"gas": expected_gas, "failed": not funded, "returnValue": "", "structLogs": []}]
        post = case["post"]
        for key in ["consensus_error", "execution_error", "parent_error", "return", "logs", "gas_used", "after_raw"]:
            assert post[key] == base[key], (case["name"], key)
        assert post["supplied_nonce"] == target["nonce"]
        accounts = json.loads(json.dumps(base["accounts"]))
        accounts[document["caller"]]["nonce"] = str(2**512 + layout + 1)
        assert post["accounts"] == accounts
        assert case["prior_raw"] == base["prior_raw"]
        assert case["quotes"] == base["quotes"] and case["runs"] == base["runs"]
        observations = ["ordered_reads", "ordered_raw_writes", "ordered_balance_effects"]
        for key in observations:
            assert case[key] == ([] if control else base[key]), (case["name"], key)
        assert case["ordered_account_scalar_reads"] == ([] if control or not funded else [{"address": target["to"], "kind": "balance", "stage": "run"}])
        expected = dict(case["prior_raw"])
        # The reduced control copies the actual post-state, with taps removed.
        for write in base["ordered_raw_writes"]:
            assert write["address"] == target["to"] and write["key"] in expected
            expected[write["key"]] = write["value"]
        assert expected == post["after_raw"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    parser.add_argument("--capture", type=Path)
    args = parser.parse_args()
    scratch = Path(os.environ.get("RUSTAXA_P2_UNDERFUNDED_TRACE_SCRATCH", "/home/fry/artifacts/claim-all-p2-underfunded-trace-producer-scratch"))
    scratch.mkdir(parents=True, exist_ok=True)
    outputs, details = {}, {}
    for control in [False, True]:
        for label in REVISIONS:
            name = label + (".control" if control else "")
            outputs[name], details[name] = run_reference(label, control, scratch)
            if args.capture:
                args.capture.mkdir(parents=True, exist_ok=True)
                for suffix, data in zip(["json", "stdout.txt", "stderr.txt"], outputs[name], strict=True):
                    args.capture.joinpath(name + "." + suffix).write_bytes(data)
                args.capture.joinpath(name + ".instrumentation.json").write_text(json.dumps(details[name], indent=2, sort_keys=True) + "\n")
    assert outputs["public"] == outputs["local"]
    assert outputs["public.control"] == outputs["local.control"]
    for label in REVISIONS:
        observed, control = json.loads(outputs[label][0]), json.loads(outputs[label + ".control"][0])
        validate(observed)
        validate(control, True)
        assert inherited.strip(observed) == inherited.strip(control)
    files = {}
    for name, output in outputs.items():
        for suffix, data in zip(["json", "stdout.txt", "stderr.txt"], output, strict=True):
            files[name + "." + suffix] = data
        files[name + ".instrumentation.json"] = (json.dumps(details[name], indent=2, sort_keys=True) + "\n").encode()
    dependencies = [Path(inherited.__file__), Path(boundary.__file__), Path(trace.__file__), Path(boundary.h1.__file__), Path(boundary.custody.__file__), Path(boundary.reward.__file__)]
    baseline_paths = [HERE / "fixtures" / name / file for name in ["native_redelegate_reward_h1", "native_reward_claim_all_underfunded", "native_reward_claim_all_trace"] for file in ["public.json", "manifest.json"]]
    manifest = {
        "schema": 1, "profile": "P2_P1_trace_seven_caps_empty_or_ordinary_prefix", "references": REVISIONS,
        "source_sha256": {str(path.relative_to(HERE)): digest(path.read_bytes()) for path in [SOURCE, Path(__file__), *SUPPORT, *dependencies]},
        "baseline_sha256": {str(path.relative_to(HERE)): digest(path.read_bytes()) for path in baseline_paths},
        "observer_helpers": {"target_prefix": digest(TARGET_HELPER), "account_scalar": digest(ACCOUNT_HELPER), "VM": digest(boundary.VM_HELPER), "read": digest(boundary.READ_HELPER), "raw": digest(boundary.custody.TRACE_HELPER), "balance": digest(boundary.reward.BALANCE_HELPER), "quote_declaration": digest(boundary.h1.QUOTE_HELPER), "post_declaration": digest(boundary.h1.POST_HELPER)},
        "copy_targets": {str(trace.TRACE_TARGET): trace.TRACE_SHA, str(boundary.STORAGE_TARGET): boundary.STORAGE_SHA, str(boundary.h1.VM_TARGET): boundary.h1.VM_SHA, str(boundary.h1.DRY_TARGET): boundary.h1.DRY_SHA, str(boundary.custody.TRACE_TARGET): boundary.custody.TRACE_SHA256, "fixture/" + DB_SOURCE.name: DB_SHA},
        "instrumented_sha256": {name: detail["instrumented_sha256"] for name, detail in details.items()},
        "outputs": {name: digest(data) for name, data in files.items()},
        "gas_caps": CAPS, "prefix_layouts": 2, "repeat_count_per_case": 2,
        "prefix_target_quote_Run_copies_in_controls": True, "control_execution_identical": True,
        "root": json.loads(outputs["public"][0])["p1_before"]["root"],
        "go_version": subprocess.check_output(["rtk", "proxy", "go", "version"], text=True).strip(),
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
    if args.record or not args.capture:
        for name, data in files.items():
            path = FIXTURES / name
            if args.record:
                path.write_bytes(data)
            else:
                assert path.read_bytes() == data, str(path)
    print("Both P2/P1 seven-cap trace pins and controls matched all14 cases; complete P1 unchanged")

if __name__ == "__main__":
 main()
