#!/usr/bin/env python3
"""Export pinned synthetic Go evidence in disposable trees; never edit the submodule.

By default compare byte-for-byte with checked-in fixtures. --record writes new
fixtures and a checksummed manifest after both references execute successfully.
Missing revisions, build failures and mismatches are fatal.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
REVISIONS = {
    "public": "6c7e5338b22d5e596cc2365a88d1f94840e1ee1b",
    "local": "bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418",
}

def run(*args, **kwargs):
    return subprocess.check_output(args, **kwargs)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {}
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-evm-reference-") as directory:
            tree = pathlib.Path(directory)
            archive = run("git", "-C", str(ROOT / "submodules/taraxa-evm"), "archive", revision)
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd/feasibility"
            command.mkdir(parents=True)
            for source in sorted(HERE.glob("reference*.go")):
                (command / source.name).write_bytes(source.read_bytes())
            artifacts[label + ".json"] = run("go", "run", "-mod=readonly", "./cmd/feasibility", cwd=tree)
    manifest = {
        "schema": 2, "references": REVISIONS,
        "go_version": run("go", "version").decode().strip(),
        "scope": "Synthetic direct EVM/state/trie fixtures; no network replay, wire admission, RocksDB or lifecycle provenance",
        "environment": {"sender": "00" * 19 + "aa", "target": "00" * 19 + "bb", "prior_nonce": "1", "period": 1, "gas_limit": 1000000, "timestamp": 0, "difficulty": "0", "value": "0", "rules": "only Cornus varies; all other flags false", "chain_config": "params.TestChainConfig at each pinned revision", "state": "only sender exists unless missing_prior_account; no code/storage; unexpected reads panic"},
        "additional_inputs": {
            "node_history": "reference_nodes.go: eight insert/update/delete/reinsert stages; reopen Go writer from prior physical nodes and root each stage; accumulated nodes and tombstones retained",
            "native_calls": "reference_native.go: six real DPoS setCommission CALL/STATICCALL cases at period1; Magnolia/Cornus, nested-call fix varies; complete five-row legacy validator storage, owner and membership; recorded code, ABI and initial raw bytes",
            "falcon": "reference_crypto.go: deterministic historical go-fn-dsa v0.2.0 SHAKE256 seeds; three keys/messages times five validity variants; Cacti FA1C ABI execution, gas cap300000 price1",
            "creation_frames": "complete account map in reference_frames.go; sender aa nonce1 balance1000000 calls bb with recorded gas_cap price1 value0, Cornus only; root from enumerated post-EVM accounts before database persistence", "opcodes": "codeInput: sender nonce 1 balance 1000000, target/child nonce 1 balance 0 with recorded code; gas cap 100000, price 1, Cornus true, Cacti varies", "native_iterable": "complete empty map at prefix 0005; one-byte items a1/b2/c3, ordered insert/remove operations; roots over Keccak(raw keys), empty bytes delete"},
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in artifacts.items()},
        "exporter_sha256": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(HERE.glob("reference*.go"))},
    }
    fixtures = HERE / "fixtures"
    if args.record:
        fixtures.mkdir(exist_ok=True)
        for name, data in artifacts.items():
            (fixtures / name).write_bytes(data)
        (fixtures / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        saved = json.loads((fixtures / "manifest.json").read_text())
        assert saved["references"] == REVISIONS
        assert saved["exporter_sha256"] == manifest["exporter_sha256"]
        for name, data in artifacts.items():
            assert data == (fixtures / name).read_bytes(), f"reference mismatch: {name}"
            assert saved["sha256"][name] == manifest["sha256"][name]
    print("Both pinned references executed; outputs " + ("recorded" if args.record else "reproduced"))
    print("Public/local identical:", artifacts["public.json"] == artifacts["local.json"])

if __name__ == "__main__":
    main()
