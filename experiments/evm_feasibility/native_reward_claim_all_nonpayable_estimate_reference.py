#!/usr/bin/env python3
"""Replay unchanged C++ estimation with accepted, measured value1 H1 guard probes."""
import argparse
import hashlib
import json
from pathlib import Path

from native_reward_claim_all_h1_estimate_reference import CPP_SOURCE, cpp_reference

HERE = Path(__file__).resolve().parent
BOUNDARIES = HERE / "fixtures/native_reward_claim_all_nonpayable"
FIXTURES = HERE / "fixtures/native_reward_claim_all_nonpayable_estimate"
CAPS = [21271, 21272, 21273, 90000, 111271, 111272, 111273]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    public = json.loads((BOUNDARIES / "public.json").read_text())
    assert public == json.loads((BOUNDARIES / "local.json").read_text())
    assert public["repeat_identical"] and public["state_unchanged_after_each"]
    assert public["state_before"] == public["state_after"]
    assert len(public["state_before"]["seed_rows"]) == 92
    rows = [row for row in public["cases"] if row["owner"] == "DryRunner.Apply"]
    assert [row["gas"] for row in rows] == CAPS
    document = {"cases": []}
    for row in rows:
        assert row["period"] == 1 and row["effective_nonce"] == "1"
        assert row["value"] == "1" and row["input"] == "0b83a727" and row["gas_price"] == "0"
        assert row["supplied_nonce"] == str(2**512)
        document["cases"].append({
            "name": "claim_all_H1_cap" + str(row["gas"]),
            "cap": row["gas"],
            "probes": [{"gas": row["gas"], "output": {
                "consensus_error": row["consensus_error"],
                "execution_error": row["execution_error"],
                "gas_used": row["gas_used"],
            }}],
        })
    cpp = cpp_reference(document)
    assert cpp["algorithm_sha256"] == "7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759"
    expected = [{"name": "claim_all_H1_cap" + str(cap), "consumed": 1,
                 "error": "intrinsic gas too low" if cap < 21272 else "Method is not payable"}
                for cap in CAPS]
    assert cpp["cases"] == expected
    manifest = {
        "schema": 1,
        "profile": "H1_initial_cap_value1_nonpayable_estimate_over_accepted_dual_pin_guard_probes",
        "references": json.loads((BOUNDARIES / "manifest.json").read_text())["references"],
        "measurement_reuse": "Exact accepted H1 boundary outputs; no new Go executions or inferred probes",
        "boundary_sha256": {path.name: digest(path) for path in sorted(BOUNDARIES.iterdir()) if path.is_file()},
        "harness_sha256": digest(Path(__file__)),
        "cpp_source_sha256": digest(CPP_SOURCE),
        "cpp_helper_sha256": digest(HERE / "native_reward_claim_all_h1_estimate_reference.py"),
        "algorithm_sha256": cpp["algorithm_sha256"],
        "probe_counts": [1] * len(CAPS),
    }
    files = {"cpp.json": cpp, "manifest.json": manifest}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
    for name, value in files.items():
        data = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()
        path = FIXTURES / name
        if args.record:
            path.write_bytes(data)
        else:
            assert path.read_bytes() == data, str(path)
    print("Seven accepted value1 H1 guard probes matched unchanged C++ estimation; one failure probe each")


if __name__ == "__main__":
    main()
