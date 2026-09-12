#!/usr/bin/env python3
"""Compile an isolated test overlay against the existing private FinalChain kernel.

Copies Rust sources into a disposable workspace, appends a test-only module and
adds test dependencies there. No source or Cargo graph in production is changed.
The native.Cargo.lock file pins the overlay graph; --record-lock refreshes it.
--clippy checks the same test composition instead of executing it.
"""
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--record-lock", action="store_true")
    mode.add_argument("--clippy", action="store_true")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="rustaxa-native-feasibility-") as directory:
        tree = pathlib.Path(directory)
        shutil.copytree(ROOT / "rust/crates", tree / "crates")
        (tree / "Cargo.toml").write_text(
            '[workspace]\nmembers=["crates/rustaxa-consensus",'
            '"crates/rustaxa-storage","crates/rustaxa-types",'
            '"crates/rustaxa-vdf"]\nresolver="3"\n'
        )
        target = tree / "crates/rustaxa-consensus/src/final_chain.rs"
        with target.open("a") as source:
            source.write(
                '\n#[cfg(test)]\n#[path = '
                + json.dumps(str(HERE / "src/native.rs"))
                + ']\nmod feasibility_native;\n'
            )
        with (tree / "crates/rustaxa-consensus/Cargo.toml").open("a") as manifest:
            manifest.write(
                '\n[dev-dependencies]\nrevm = {'
                'git="https://github.com/bluealloy/revm.git",'
                'rev="6014612c86f3690e4e9173a8c4deade396af398d",'
                'default-features=false,features=["std"]}\nhex="0.4"\n'
            )
        lock = HERE / "native.Cargo.lock"
        if lock.exists():
            shutil.copy(lock, tree / "Cargo.lock")
        env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / "rust/target"))
        cmd = ["cargo", "clippy" if args.clippy else "test"]
        if not args.record_lock:
            cmd.append("--locked")
        cmd.extend(["--manifest-path", str(tree / "Cargo.toml"), "-p", "rustaxa-consensus"])
        if args.clippy:
            cmd.append("--tests")
        else:
            cmd.extend(["feasibility_native", "--", "--nocapture"])
        subprocess.run(cmd, env=env, check=True)
        if args.record_lock:
            shutil.copy(tree / "Cargo.lock", lock)


if __name__ == "__main__":
    main()
