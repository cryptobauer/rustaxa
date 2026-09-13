#!/usr/bin/env python3
"""Build a tiny mainnet-genesis identity reader from an existing CMake tree.

The script reads the configured taraxad compile/link commands but writes the
helper object and executable only to the requested path. It neither configures
nor builds the shared CMake tree and never opens snapshot data.
"""

import argparse
import pathlib
import shlex
import subprocess

from snapshot_manifest import validate_outputs


HERE = pathlib.Path(__file__).resolve().parent
SOURCE = HERE / "snapshot_mainnet_genesis.cpp"


def make_value(path, name):
    prefix = name + " = "
    for line in path.read_text().splitlines():
        if line.startswith(prefix):
            return shlex.split(line[len(prefix):])
    raise RuntimeError(f"missing {name} in {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build", default="/build")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    build = pathlib.Path(args.build).resolve(strict=True)
    target = build / "programs/taraxad/CMakeFiles/taraxad.dir"
    flags_path = target / "flags.make"
    link_path = target / "link.txt"
    output = pathlib.Path(args.output).absolute()
    object_path = output.with_suffix(".o")
    _, (output, object_path) = validate_outputs(
        [build, HERE, pathlib.Path("/tmp/snapshot-litenode")], [output, object_path]
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    # Reserve both outputs exclusively before invoking tools that overwrite -o paths.
    with output.open("xb"), object_path.open("xb"):
        pass

    link = shlex.split(link_path.read_text())
    compiler = link[0]
    compile_command = [
        compiler,
        *make_value(flags_path, "CXX_DEFINES"),
        *make_value(flags_path, "CXX_INCLUDES"),
        *make_value(flags_path, "CXX_FLAGS"),
        "-std=c++20",
        "-c",
        str(SOURCE),
        "-o",
        str(object_path),
    ]
    subprocess.run(compile_command, check=True)

    rewritten = [compiler]
    index = 1
    while index < len(link):
        token = link[index]
        if token == "-Xlinker" and index + 1 < len(link) and link[index + 1].startswith("--dependency-file="):
            index += 2
            continue
        if token == "CMakeFiles/taraxad.dir/main.cpp.o":
            rewritten.append(str(object_path))
        elif token == "-o":
            rewritten.extend([token, str(output)])
            index += 2
            continue
        else:
            rewritten.append(token)
        index += 1
    subprocess.run(rewritten, cwd=build / "programs/taraxad", check=True)
    subprocess.run([str(output)], check=True)


if __name__ == "__main__":
    main()
