#!/usr/bin/env python3
"""Measure/reproduce actual dual-pin H1/P2 claim-all gas boundaries and controls."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from reference import REVISIONS, ROOT
import native_reward_claim_all_h1_reference as h1
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody

HERE = Path(__file__).resolve().parent
SOURCE = HERE / "native_reward_claim_all_underfunded_reference.go"
SUPPORT = [*h1.SUPPORT, h1.SOURCE]
FIXTURES = HERE / "fixtures/native_reward_claim_all_underfunded"
STORAGE_TARGET = Path("taraxa/state/contracts/storage/storage.go")
STORAGE_SHA = "2f56b13597e9b24e022e496a2be650c65938f951e619ab958a986a24971d7d2c"
LOGICAL_NEEDLE = b"func (self *StorageReaderWrapper) Get(k *common.Hash, cb func([]byte)) {\n"
BACKEND_NEEDLE = b"\tself.StorageReader.GetAccountStorage(self.contract_address, k, func(bytes []byte) {\n"
READ_HELPER = b'''package contract_storage
import "github.com/Taraxa-project/taraxa-evm/common"
var claimAllBoundaryReadObserver func(string,*common.Address,*common.Hash)
func SetClaimAllBoundaryReadObserver(observer func(string,*common.Address,*common.Hash)){claimAllBoundaryReadObserver=observer}
func observeClaimAllBoundaryRead(kind string,address *common.Address,key *common.Hash){if claimAllBoundaryReadObserver!=nil{claimAllBoundaryReadObserver(kind,address,key)}}
'''
RUN_NEEDLE = b"precompiled.Run(frame, self)"
VM_HELPER = b'''package vm
var claimAllBoundaryObserver func(string,bool,CallFrame,*EVM,uint64,[]byte,error)
func SetClaimAllBoundaryObserver(observer func(string,bool,CallFrame,*EVM,uint64,[]byte,error)){claimAllBoundaryObserver=observer}
func observeClaimAllBoundary(kind string,start bool,frame CallFrame,e *EVM,gas uint64,out []byte,err error){if claimAllBoundaryObserver!=nil{claimAllBoundaryObserver(kind,start,frame,e,gas,out,err)}}
func claimAllBoundaryRequired(p PrecompiledContract,frame CallFrame,e *EVM)uint64{
 observeClaimAllBoundary("quote",true,frame,e,0,nil,nil)
 required:=p.RequiredGas(frame,e)
 observeClaimAllBoundary("quote",false,frame,e,required,nil,nil)
 return required
}
func claimAllBoundaryRun(p PrecompiledContract,frame CallFrame,e *EVM)([]byte,error){
 observeClaimAllBoundary("run",true,frame,e,0,nil,nil)
 out,err:=p.Run(frame,e)
 observeClaimAllBoundary("run",false,frame,e,0,out,err)
 return out,err
}
'''
OBSERVATIONS = {"ordered_reads", "ordered_raw_writes", "ordered_balance_effects"}
CAPS = [21271, 21272, 21273, 90000, 111271, 111272, 111273]
# Exact ordered quote keys from the first dual-pin actual capture, including
# lazy setup before the caller count; never relabel these as absent reads.
QUOTE_KEYS = [
    "f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393",
    "dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795",
    "ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc",
    "d0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873",
    "4ea47a5db4abbbddf0d3f15bf4c5c706e9c788fa5d2b06349c6084aeddb0de92",
]



def digest(data):
    return hashlib.sha256(data).hexdigest()


def replace_once(data, needle, replacement):
    if data.count(needle) != 1:
        raise RuntimeError("Copy observer source shape changed: " + repr(needle))
    return data.replace(needle, replacement, 1)


def run_reference(label, control, scratch):
    # Persistent scratch holds a resumable unique producer tree until completion.
    with tempfile.TemporaryDirectory(prefix=label + "-", dir=scratch) as directory:
        tree = Path(directory)
        archive = subprocess.check_output([
            "rtk", "proxy", "git", "-C", str(ROOT / "submodules/taraxa-evm"), "archive", REVISIONS[label]
        ])
        subprocess.run(["rtk", "proxy", "tar", "-x", "-C", directory], input=archive, check=True)
        guards = [(custody.TRACE_TARGET, custody.TRACE_SHA256), (h1.DRY_TARGET, h1.DRY_SHA),
                  (h1.VM_TARGET, h1.VM_SHA[label]), (STORAGE_TARGET, STORAGE_SHA)]
        for path, expected in guards:
            if digest((tree / path).read_bytes()) != expected:
                raise RuntimeError("Copy target hash changed: " + str(path))
        target = tree / custody.TRACE_TARGET
        data = target.read_bytes()
        # Both controls and observed runs retain helper declarations. Controls
        # omit account/raw taps; the execution and post-state must still match.
        for operation, expression in [("add", "Add"), ("sub", "Sub")]:
            needle = ("self.set_balance(new(big.Int)." + expression + "(self.Balance, amount))").encode()
            insertion = needle if control else needle + b'\n\t\tobserveRewardBalance(self.addr,"' + operation.encode() + b'",amount,self.Balance)'
            data = replace_once(data, needle, insertion)
        data = replace_once(data, custody.TRACE_NEEDLE, custody.TRACE_NEEDLE if control else custody.TRACE_INSERTION)
        target.write_bytes(data)
        (target.parent / "native_v1_custody_observer.go").write_bytes(custody.TRACE_HELPER)
        (target.parent / "reward_balance_observer.go").write_bytes(reward.BALANCE_HELPER)
        target = tree / h1.DRY_TARGET
        target.write_bytes(replace_once(target.read_bytes(), h1.DRY_NEEDLE, h1.DRY_NEEDLE + b"\tobserveRewardH1Post(block_state)\n"))
        (target.parent / "reward_h1_post_observer.go").write_bytes(h1.POST_HELPER)
        target = tree / h1.VM_TARGET
        data = replace_once(target.read_bytes(), h1.VM_NEEDLE, b"claimAllBoundaryRequired(precompiled, frame, self)")
        target.write_bytes(replace_once(data, RUN_NEEDLE, b"claimAllBoundaryRun(precompiled, frame, self)"))
        (target.parent / "claim_all_boundary_observer.go").write_bytes(VM_HELPER)
        # The accepted H1 support's unused main still references this declaration.
        (target.parent / "reward_h1_quote_observer.go").write_bytes(h1.QUOTE_HELPER)
        target = tree / STORAGE_TARGET
        data = replace_once(target.read_bytes(), LOGICAL_NEEDLE, LOGICAL_NEEDLE if control else LOGICAL_NEEDLE + b'\tobserveClaimAllBoundaryRead("logical",self.contract_address,k)\n')
        data = replace_once(data, BACKEND_NEEDLE, BACKEND_NEEDLE if control else b'\tobserveClaimAllBoundaryRead("backend",self.contract_address,k)\n' + BACKEND_NEEDLE)
        target.write_bytes(data)
        (target.parent / "claim_all_boundary_read_observer.go").write_bytes(READ_HELPER)
        command = tree / "cmd/claim_all_boundary"
        command.mkdir(parents=True)
        for index, path in enumerate(SUPPORT):
            data = path.read_bytes()
            (command / path.name).write_bytes(replace_once(data, b"func main() {", f"func unusedBoundarySupport{index}Main() {{".encode()))
        (command / SOURCE.name).write_bytes(SOURCE.read_bytes())
        result = subprocess.run(["rtk", "proxy", "go", "run", "-mod=readonly", "./cmd/claim_all_boundary"],
                                cwd=tree, capture_output=True,
                                env={**os.environ, "CLAIM_ALL_BOUNDARY_CONTROL": "1" if control else "0"})
        if result.returncode:
            sys.stdout.buffer.write(result.stdout)
            sys.stderr.buffer.write(result.stderr)
            result.check_returncode()
        lines = result.stdout.splitlines(keepends=True)
        if not lines:
            raise RuntimeError("Missing boundary output")
        return lines[-1], b"".join(lines[:-1]), result.stderr


def strip(value):
    if isinstance(value, dict):
        return {key: strip(item) for key, item in value.items() if key not in OBSERVATIONS}
    if isinstance(value, list):
        return [strip(item) for item in value]
    return value


def validate(document, control=False):
    seed = json.loads((HERE / "fixtures/native_redelegate_reward_h1/public.json").read_text())
    funded_h1 = json.loads((HERE / "fixtures/native_reward_claim_all_h1/public.json").read_text())["cases"][0]
    funded_p2 = json.loads((HERE / "fixtures/native_reward_claim_all/public.json").read_text())["cases"][0]
    assert document["schema"] == 1
    assert document["state_before"] == document["state_after"] == seed["state_before"]
    assert len(document["state_before"]["seed_rows"]) == 92
    assert document["repeat_identical"] and document["state_unchanged_after_each"]
    assert [(case["period"], case["gas"]) for case in document["cases"]] == [(period, cap) for period in (1, 2) for cap in CAPS]
    for case in document["cases"]:
        cap = case["gas"]
        assert case["supplied_nonce"] == (str(2**512) if case["period"] == 1 else "0")
        assert case["gas_price"] == case["value"] == "0" and case["input"] == "0b83a727"
        assert case["effective_nonce"] == ("1" if case["period"] == 1 else "0")
        assert case["owner"] == ("DryRunner.Apply" if case["period"] == 1 else "EVM.Main")
        assert case["quote_called"] == (cap >= 21272) and case["run_called"] == (cap >= 111272)
        assert len(case["quotes"]) == int(cap >= 21272) and len(case["runs"]) == int(cap >= 111272)
        if cap >= 21272:
            quote = case["quotes"][0]
            assert quote["required_gas"] == 90000 and quote["supplied_native_gas"] == cap - 21272
            assert quote["input"] == "0b83a727" and quote["caller"] == document["caller"]
            assert quote["depth"] == 0 and quote["value"] == "0"
        if cap < 111272:
            assert not case["ordered_raw_writes"] and not case["ordered_balance_effects"]
            assert case["after_raw"] == case["prior_raw"]
            assert case["return"] == "" and not case["logs"]
            assert case["accounts"][document["caller"]]["balance"] == "3000"
            assert case["accounts"][case["to"]]["balance"] == "4800"
            if not control:
                reads = case["ordered_reads"]
                assert len(reads) == (0 if cap < 21272 else 10), (case["period"], cap, reads)
                if reads:
                    assert [read["kind"] for read in reads] == ["logical", "backend"] * 5
                    assert all(read["stage"] == "quote" for read in reads)
                    assert [read["key"] for read in reads] == [key for key in QUOTE_KEYS for _ in range(2)]
                    assert all(read["address"] == case["to"] for read in reads)
        elif not control:
            assert len(case["ordered_raw_writes"]) == 12 and len(case["ordered_balance_effects"]) == 2
            positive = funded_h1 if case["period"] == 1 else funded_p2
            assert case["ordered_raw_writes"] == positive["ordered_raw_writes"]
            assert case["ordered_balance_effects"] == positive["ordered_balance_effects"]
            assert case["logs"] == (positive["output"]["logs"] if case["period"] == 1 else positive["logs"])
            assert len(case["ordered_reads"]) == 48
            assert [read["key"] for read in case["ordered_reads"][:10]] == [key for key in QUOTE_KEYS for _ in range(2)]
            assert all(read["stage"] == "quote" for read in case["ordered_reads"][:10])
            assert all(read["stage"] == "run" for read in case["ordered_reads"][10:])
            expected = dict(case["prior_raw"])
            for write in case["ordered_raw_writes"]:
                assert write["key"] in expected and write["address"] == case["to"]
                expected[write["key"]] = write["value"]
            assert expected == case["after_raw"]
        # Exact lanes from both actual pins, including the outer nonce on failure.
        assert case["gas_used"] == (21271 if cap < 21272 else 21272 if cap < 111272 else 111272)
        assert case["consensus_error"] == ("intrinsic gas too low" if cap < 21272 else "")
        assert case["execution_error"] == ("out of gas" if 21272 <= cap < 111272 else "")
        assert case["parent_error"] == (None if case["period"] == 1 else "out of gas" if 21272 <= cap < 111272 else "")
        assert case["accounts"][document["caller"]]["nonce"] == ("2" if case["period"] == 1 else "1")
        assert case["intrinsic_gas"] == 21272
        assert case["native_gas_used"] == (None if cap < 21272 else 0 if cap < 111272 else 90000)
        assert case["native_gas_left"] == (None if cap < 21272 else cap - case["gas_used"])
        if cap >= 111272:
            assert case["runs"] == [{"supplied_native_gas": cap - 21272, "error": "", "output": ""}]
            assert case["accounts"][document["caller"]]["balance"] == "3396"
            assert case["accounts"][case["to"]]["balance"] == "4404"
            assert len(case["logs"]) == 1 and case["return"] == ""



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    args.scratch.mkdir(parents=True, exist_ok=True)
    outputs = {label: run_reference(label, False, args.scratch) for label in REVISIONS}
    controls = {label: run_reference(label, True, args.scratch) for label in REVISIONS}
    args.evidence.mkdir(parents=True, exist_ok=False)
    for label, result in {**outputs, **{label + ".control": result for label, result in controls.items()}}.items():
        for suffix, content in zip(["json", "stdout.txt", "stderr.txt"], result):
            (args.evidence / (label + "." + suffix)).write_bytes(content)
    assert outputs["public"] == outputs["local"] and controls["public"] == controls["local"]
    for label in REVISIONS:
        document, control = json.loads(outputs[label][0]), json.loads(controls[label][0])
        validate(document)
        validate(control, True)
        assert strip(document) == strip(control)
    manifest = {"schema": 1, "references": REVISIONS, "caps": CAPS,
                "profile": "canonical92P1_H1_DryRunner_pending2_Main_lazy_setup_then_count_gas_terminal",
                "go_version": subprocess.check_output(["rtk", "proxy", "go", "version"], text=True).strip(),
                "source_sha256": {path.name: digest(path.read_bytes()) for path in [SOURCE, Path(__file__), *SUPPORT]},
                "guard_source_sha256": {Path(module.__file__).name: digest(Path(module.__file__).read_bytes()) for module in [h1, reward, custody]},
                "baseline_sha256": {name: digest((HERE / name).read_bytes()) for name in ["fixtures/native_redelegate_reward_h1/public.json", "fixtures/native_reward_claim_all_h1/public.json", "fixtures/native_reward_claim_all/public.json"]},
                "copy_targets": {str(custody.TRACE_TARGET): custody.TRACE_SHA256, str(h1.DRY_TARGET): h1.DRY_SHA,
                                 str(h1.VM_TARGET): h1.VM_SHA, str(STORAGE_TARGET): STORAGE_SHA},
                "copy_helpers": {"vm": digest(VM_HELPER), "read": digest(READ_HELPER), "post": digest(h1.POST_HELPER),
                                 "balance": digest(reward.BALANCE_HELPER), "raw": digest(custody.TRACE_HELPER)},
                "control_execution_identical": True, "quote_and_run_taps_in_controls": True,
                "native_gas_derivation": "actual Main transaction gas used minus measured intrinsic21272 for direct first native; leftover is transaction cap minus actual gas used; null below intrinsic",
                "read_lanes": "logical wrapper requests plus backend requests, only during quote/Run; post-state and outer envelope reads excluded"}
    combined = {**outputs, **{label + ".control": result for label, result in controls.items()}}
    manifest["outputs"] = {label: [digest(part) for part in result] for label, result in combined.items()}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        (FIXTURES / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    else:
        assert json.loads((FIXTURES / "manifest.json").read_text()) == manifest
    for label, result in combined.items():
        for suffix, content in zip(["json", "stdout.txt", "stderr.txt"], result):
            path = FIXTURES / (label + "." + suffix)
            if args.record:
                path.write_bytes(content)
            else:
                assert path.read_bytes() == content, str(path)
    print("Both pins executed all14 H1/P2 boundaries twice; controls and committed92-rowP1 matched")


if __name__ == "__main__":
    main()
