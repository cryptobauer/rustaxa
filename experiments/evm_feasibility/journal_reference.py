#!/usr/bin/env python3
"""Reproduce pinned journal/TrieSink contracts without opening a node database.

--record writes only this additive corpus after both pinned references agree.
The original research corpus remains unchanged. Input programs and reference
identities are recorded alongside content hashes; memory rows are not RocksDB
reopen or retained-history evidence.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = pathlib.Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {}
    source = HERE / "journal_reference.go"
    for label, revision in REVISIONS.items():
        with tempfile.TemporaryDirectory(prefix="rustaxa-journal-reference-") as directory:
            tree = pathlib.Path(directory)
            archive = subprocess.check_output(
                ["git", "-C", str(ROOT / "submodules/taraxa-evm"), "archive", revision]
            )
            subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
            command = tree / "cmd/journal-contracts"
            command.mkdir(parents=True)
            (command / "main.go").write_bytes(source.read_bytes())
            artifacts[f"journal_{label}.json"] = subprocess.check_output(
                ["go", "run", "-mod=readonly", "./cmd/journal-contracts"], cwd=tree
            )
    if artifacts["journal_public.json"] != artifacts["journal_local.json"]:
        raise RuntimeError("pinned journal references disagree")
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"], text=True).strip(),
        "scope": "Ten synthetic journal/physical TrieSink cases; no EVM envelope, RocksDB or network replay",
        "exporter_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in artifacts.items()},
    }
    fixtures = HERE / "fixtures"
    manifest_path = fixtures / "journal_manifest.json"
    if args.record:
        for name, data in artifacts.items():
            (fixtures / name).write_bytes(data)
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        if json.loads(manifest_path.read_text()) != manifest:
            raise RuntimeError("journal manifest mismatch")
        for name, data in artifacts.items():
            if (fixtures / name).read_bytes() != data:
                raise RuntimeError(f"journal fixture mismatch: {name}")
    print("Both pinned journal/TrieSink references agree; " + ("recorded" if args.record else "reproduced"))


if __name__ == "__main__":
    main()
