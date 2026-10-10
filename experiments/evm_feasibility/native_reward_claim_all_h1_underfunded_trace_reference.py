#!/usr/bin/env python3
"""Measure seven literal H1/P0 trace gas caps with inherited pinned instrumentation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import native_reward_claim_all_h1_trace_reference as trace

HERE = Path(__file__).resolve().parent
SOURCE = HERE / "native_reward_claim_all_h1_underfunded_trace_reference.go"
FIXTURES = HERE / "fixtures/native_reward_claim_all_h1_underfunded_trace"
CAPS = [21271, 21272, 21273, 90000, 111271, 111272, 111273]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def validate(document, control=False):
    accepted = json.loads((trace.FIXTURES / "public.json").read_text())
    assert document["p0_capture_count"] == 1
    assert document["repeat_identical"] and document["source_unchanged_after_each"]
    assert document["p0_source_before"] == document["p0_source_after"] == accepted["p0_source_before"]
    assert document["p1_before"] == document["p1_after"] == accepted["p1_before"]
    assert len(document["cases"]) == len(CAPS)
    for cap, case in zip(CAPS, document["cases"], strict=True):
        assert case["name"] == "claim_all_H1_P0_gas" + str(cap)
        assert not case["prefix"] and case["reader_periods"] == [0] and case["execution_period"] == 1
        assert len(case["targets"]) == len(case["result"]) == 1
        target = case["targets"][0]
        assert target == {**accepted["cases"][0]["targets"][0], "gas": cap}
        assert case["post"]["supplied_nonce"] == str(2**512)
        assert case["prior_raw"] == accepted["cases"][0]["prior_raw"]
        assert case["ordered_account_scalar_reads"] == case["ordered_balance_effects"] == []
        # Outcomes below were encoded only after the saved actual four-run capture.
        funded = cap >= 111272
        intrinsic = cap == 21271
        expected_gas = 111272 if funded else 21271 if intrinsic else 21272
        assert case["diagnostics"] == ""
        assert case["result"] == [{"gas": expected_gas, "failed": not funded, "returnValue": "", "structLogs": []}]
        post = case["post"]
        assert post["gas_used"] == expected_gas and post["return"] == "" and post["logs"] == []
        assert post["consensus_error"] == ("intrinsic gas too low" if intrinsic else "")
        assert post["execution_error"] == post["parent_error"] == ("out of gas" if not funded and not intrinsic else "")
        assert post["accounts"] == accepted["cases"][0]["post"]["accounts"]
        assert post["after_raw"] == (accepted["cases"][0]["post"]["after_raw"] if funded else case["prior_raw"])
        quote = {**accepted["cases"][0]["quotes"][0], "supplied_native_gas": cap-21272}
        assert case["quotes"] == ([] if intrinsic else [quote])
        assert case["runs"] == ([{"supplied_native_gas": cap-21272, "output": "", "error": ""}] if funded else [])
        if not control:
            assert case["ordered_reads"] == (accepted["cases"][0]["ordered_reads"] if funded else [] if intrinsic else accepted["cases"][0]["ordered_reads"][:10])
            assert case["ordered_raw_writes"] == (accepted["cases"][0]["ordered_raw_writes"] if funded else [])
        if control:
            assert case["ordered_reads"] == case["ordered_raw_writes"] == []
        else:
            expected = dict(case["prior_raw"])
            for write in case["ordered_raw_writes"]:
                assert write["address"] == target["to"] and write["key"] in expected
                expected[write["key"]] = write["value"]
            assert expected == case["post"]["after_raw"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    parser.add_argument("--capture", type=Path, help="Preserve first measured outputs before validation")
    args = parser.parse_args()
    scratch = Path(os.environ.get("RUSTAXA_H1_UNDERFUNDED_TRACE_SCRATCH", "/home/fry/artifacts/claim-all-h1-underfunded-trace-producer-scratch"))
    scratch.mkdir(parents=True, exist_ok=True)
    trace.SOURCE = SOURCE
    outputs, details = {}, {}
    for control in [False, True]:
        for label in trace.REVISIONS:
            name = label + (".control" if control else "")
            outputs[name], details[name] = trace.run_reference(label, control, scratch)
            if args.capture:
                args.capture.mkdir(parents=True, exist_ok=True)
                for suffix, data in zip(["json", "stdout.txt", "stderr.txt"], outputs[name], strict=True):
                    args.capture.joinpath(name + "." + suffix).write_bytes(data)
                args.capture.joinpath(name + ".instrumentation.json").write_text(json.dumps(details[name], indent=2, sort_keys=True) + "\n")
    assert outputs["public"] == outputs["local"]
    assert outputs["public.control"] == outputs["local.control"]
    for label in trace.REVISIONS:
        observed, control = json.loads(outputs[label][0]), json.loads(outputs[label + ".control"][0])
        validate(observed)
        validate(control, True)
        assert trace.strip(observed) == trace.strip(control)
    files = {}
    for name, output in outputs.items():
        for suffix, data in zip(["json", "stdout.txt", "stderr.txt"], output, strict=True):
            files[name + "." + suffix] = data
        files[name + ".instrumentation.json"] = (json.dumps(details[name], indent=2, sort_keys=True) + "\n").encode()
    inherited = [Path(trace.__file__), Path(trace.boundary.__file__), Path(trace.trace.__file__), Path(trace.boundary.h1.__file__), Path(trace.boundary.custody.__file__), Path(trace.boundary.reward.__file__)]
    manifest = {
        "schema": 1, "profile": "literal_H1_P0_trace_seven_measured_gas_caps", "references": trace.REVISIONS,
        "source_sha256": {path.name: digest(path.read_bytes()) for path in [SOURCE, Path(__file__), *trace.SUPPORT, *inherited]},
        "accepted_H1_P0_oracle_sha256": digest((trace.FIXTURES / "public.json").read_bytes()),
        "accepted_H1_P0_manifest_sha256": digest((trace.FIXTURES / "manifest.json").read_bytes()),
        "instrumented_sha256": {name: detail["instrumented_sha256"] for name, detail in details.items()},
        "outputs": {name: digest(data) for name, data in files.items()},
        "gas_caps": CAPS, "repeat_count_per_cap": 2,
        "P0_capture_and_quote_run_target_copies_in_controls": True,
        "control_execution_identical": True,
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
    print("Both actual seven-cap H1/P0 trace pins and controls matched; full P0 and P1 remained unchanged")


if __name__ == "__main__":
    main()
