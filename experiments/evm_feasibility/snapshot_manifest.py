#!/usr/bin/env python3
"""Create or compare deterministic full-content manifests for snapshot trees.

The JSONL manifest stays outside Git. Each line binds one relative path to its
file size and SHA-256. The final summary binds the ordered records to a single
SHA-256 and is small enough to include in qualification evidence.
"""

import argparse
import hashlib
import json
import os
import pathlib


def files(root):
    pending = [pathlib.Path(root)]
    while pending:
        directory = pending.pop()
        entries = sorted(os.scandir(directory), key=lambda entry: entry.name)
        for entry in entries:
            path = pathlib.Path(entry.path)
            if entry.is_dir(follow_symlinks=False):
                pending.append(path)
            elif entry.is_file(follow_symlinks=False):
                yield path
            else:
                raise RuntimeError(f"unsupported snapshot entry: {path}")


def manifest(root, output):
    root = pathlib.Path(root).resolve(strict=True)
    count = 0
    byte_count = 0
    aggregate = hashlib.sha256()
    with pathlib.Path(output).open("wb") as destination:
        for path in sorted(files(root), key=lambda item: item.relative_to(root).as_posix()):
            digest = hashlib.sha256()
            with path.open("rb") as source:
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(chunk)
            record = {
                "path": path.relative_to(root).as_posix(),
                "bytes": path.stat().st_size,
                "sha256": digest.hexdigest(),
            }
            encoded = json.dumps(record, sort_keys=True, separators=(",", ":")).encode() + b"\n"
            destination.write(encoded)
            aggregate.update(encoded)
            count += 1
            byte_count += record["bytes"]
    return {"regular_files": count, "logical_bytes": byte_count, "manifest_sha256": aggregate.hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source")
    parser.add_argument("source_manifest")
    parser.add_argument("--compare")
    parser.add_argument("--compare-manifest")
    args = parser.parse_args()
    source = manifest(args.source, args.source_manifest)
    result = {"source": source}
    if bool(args.compare) != bool(args.compare_manifest):
        parser.error("--compare and --compare-manifest must be supplied together")
    if args.compare:
        compared = manifest(args.compare, args.compare_manifest)
        result["compared"] = compared
        result["equal"] = source == compared
        if not result["equal"]:
            raise SystemExit(json.dumps(result, sort_keys=True))
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
