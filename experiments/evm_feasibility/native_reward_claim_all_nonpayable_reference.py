#!/usr/bin/env python3
"""Measure/reproduce bounded value1 Cornus claim-all nonpayable gas guards."""
import argparse
import json
from pathlib import Path
import subprocess
import native_reward_claim_all_underfunded_reference as boundary
from reference import REVISIONS

HERE = Path(__file__).resolve().parent
SOURCE = HERE / "native_reward_claim_all_nonpayable_reference.go"
FIXTURES = HERE / "fixtures/native_reward_claim_all_nonpayable"
SUPPORT = boundary.SUPPORT
h1, reward, custody = boundary.h1, boundary.reward, boundary.custody
CAPS = boundary.CAPS
digest, strip = boundary.digest, boundary.strip
STORAGE_TARGET, STORAGE_SHA = boundary.STORAGE_TARGET, boundary.STORAGE_SHA
VM_HELPER, READ_HELPER = boundary.VM_HELPER, boundary.READ_HELPER


def run_reference(label, control, scratch):
    # Reuse the accepted, hash-guarded copy instrumentation without changing it.
    boundary.SOURCE = SOURCE
    return boundary.run_reference(label, control, scratch)


def validate(document, control=False):
    seed = json.loads((HERE / "fixtures/native_redelegate_reward_h1/public.json").read_text())
    previous = json.loads((HERE / "fixtures/native_reward_claim_all_underfunded/public.json").read_text())
    assert document["schema"] == 1
    assert document["state_before"] == document["state_after"] == seed["state_before"]
    assert len(document["state_before"]["seed_rows"]) == 92
    assert document["repeat_identical"] and document["state_unchanged_after_each"]
    assert [(c["period"], c["gas"]) for c in document["cases"]] == [(p, g) for p in (1, 2) for g in CAPS]
    assert document["caller"] == previous["caller"]
    for case, prior in zip(document["cases"], previous["cases"], strict=True):
        cap, period = case["gas"], case["period"]
        admitted = cap >= 21272
        assert case["supplied_nonce"] == (str(2**512) if period == 1 else "0")
        assert case["effective_nonce"] == ("1" if period == 1 else "0")
        assert case["gas_price"] == "0" and case["value"] == "1" and case["input"] == "0b83a727"
        assert case["owner"] == ("DryRunner.Apply" if period == 1 else "EVM.Main")
        assert case["quote_called"] == case["run_called"] == admitted
        assert len(case["quotes"]) == len(case["runs"]) == int(admitted)
        assert case["prior_raw"] == prior["prior_raw"] == case["after_raw"]
        assert len(case["prior_raw"]) == 23
        expected_accounts = previous["cases"][0 if period == 1 else 7]["accounts"]
        assert case["accounts"] == expected_accounts  # all three accounts, including outer nonce
        assert not case["ordered_raw_writes"] and not case["ordered_balance_effects"]
        assert not case["logs"] and case["return"] == ""
        assert case["gas_used"] == (21272 if admitted else 21271)
        assert case["consensus_error"] == ("" if admitted else "intrinsic gas too low")
        assert case["execution_error"] == ("Method is not payable" if admitted else "")
        assert case["parent_error"] == (None if period == 1 else "Method is not payable" if admitted else "")
        assert case["intrinsic_gas"] == 21272
        assert case["native_gas_used"] == (0 if admitted else None)
        assert case["native_gas_left"] == (cap - 21272 if admitted else None)
        if admitted:
            assert case["quotes"] == [{
                "required_gas": 0, "supplied_native_gas": cap - 21272,
                "caller": document["caller"], "depth": 0, "value": "1", "input": "0b83a727",
                "frame_accounts": {"caller_balance": "2999", "native_balance": "4801", "caller_nonce": "2" if period == 1 else "1"},
            }]
            assert case["runs"] == [{"supplied_native_gas": cap - 21272, "error": "Method is not payable", "output": ""}]
        expected_reads = [] if control or not admitted else [
            {"kind": kind, "key": key, "address": case["to"], "stage": "quote"}
            for key in boundary.QUOTE_KEYS[:4] for kind in ("logical", "backend")
        ]
        assert case["ordered_reads"] == expected_reads
        assert not any(read["key"] == boundary.QUOTE_KEYS[4] for read in case["ordered_reads"])


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
                "profile": "canonical92P1_Cornus_value1_H1_DryRunner_pending2_Main_nonpayable_guard",
                "go_version": subprocess.check_output(["rtk", "proxy", "go", "version"], text=True).strip(),
                "source_sha256": {path.name: digest(path.read_bytes()) for path in [SOURCE, Path(__file__), *SUPPORT]},
                "guard_source_sha256": {Path(module.__file__).name: digest(Path(module.__file__).read_bytes()) for module in [boundary, h1, reward, custody]},
                "baseline_sha256": {name: digest((HERE / name).read_bytes()) for name in ["fixtures/native_redelegate_reward_h1/public.json", "fixtures/native_reward_claim_all_h1/public.json", "fixtures/native_reward_claim_all/public.json", "fixtures/native_reward_claim_all_underfunded/public.json"]},
                "copy_targets": {str(custody.TRACE_TARGET): custody.TRACE_SHA256, str(h1.DRY_TARGET): h1.DRY_SHA,
                                 str(h1.VM_TARGET): h1.VM_SHA, str(STORAGE_TARGET): STORAGE_SHA},
                "copy_helpers": {"vm": digest(VM_HELPER), "read": digest(READ_HELPER), "post": digest(h1.POST_HELPER),
                                 "balance": digest(reward.BALANCE_HELPER), "raw": digest(custody.TRACE_HELPER)},
                "control_execution_identical": True, "quote_frame_account_taps_in_controls": True,
                "value_transfer": "actual frame before quote caller2999/native4801; all three final accounts match source plus measured envelope nonce after failure rollback",
                "native_read_boundary": "lazy bootstrap four logical/backend pairs; no caller count; qualified Rust omission is a separate lane", "quote_and_run_taps_in_controls": True,
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
    print("Both pins executed all14 value1 H1/P2 guards twice; controls, transfer rollback and committed92-rowP1 matched")


if __name__ == "__main__":
    main()
