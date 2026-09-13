#!/usr/bin/env python3
"""Reproduce bounded ordered observer fixtures in both pinned Go trees."""

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
FIXTURES = HERE / "fixtures"
EXPORTER = HERE / "ordered_overlay_reference.go"


def execute(revision: str) -> bytes:
    with tempfile.TemporaryDirectory(prefix="rustaxa-ordered-overlay-") as directory:
        tree = pathlib.Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", revision])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        command = tree / "cmd/ordered_overlay_reference"
        command.mkdir(parents=True)
        (command / "main.go").write_bytes(EXPORTER.read_bytes())
        return subprocess.check_output(
            ["go", "run", "-mod=readonly", "./cmd/ordered_overlay_reference"], cwd=tree
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    artifacts = {label: execute(revision) for label, revision in REVISIONS.items()}
    if artifacts["public"] != artifacts["local"]:
        raise RuntimeError("pinned ordered observer outputs disagree")
    manifest = {
        "schema": 1,
        "references": REVISIONS,
        "go_version": subprocess.check_output(["go", "version"], text=True).strip(),
        "scope": (
            "Synthetic direct TrieSink ordered ordinary/raw and account delete/recreate "
            "phases with exact accumulated memory rows; no EVM envelope, RocksDB, "
            "application publication, imported history, or production-routing claim"
        ),
        "exporter_sha256": hashlib.sha256(EXPORTER.read_bytes()).hexdigest(),
        "sha256": {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()},
    }
    fixture = FIXTURES / "ordered_overlay.json"
    manifest_path = FIXTURES / "ordered_overlay_manifest.json"
    if args.record:
        fixture.write_bytes(artifacts["public"])
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    else:
        if json.loads(manifest_path.read_text()) != manifest:
            raise RuntimeError("saved ordered observer metadata differs")
        if fixture.read_bytes() != artifacts["public"]:
            raise RuntimeError("saved ordered observer fixture differs")
    print("Both pinned ordered observer references executed and " + ("recorded" if args.record else "reproduced"))


if __name__ == "__main__":
    main()
