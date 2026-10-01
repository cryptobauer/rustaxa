#!/usr/bin/env python3
"""Run the pinned synthetic cold lifecycle; persist archive and evidence safely."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from empty_native_effects_reference import REVISION, SOURCE, SOURCE_PINS, patch_archive
from current_rewards_reference import TRACE_TARGET, TRACE_SHA256, TRACE_NEEDLE, TRACE_INSERTION, TRACE_HELPER

HERE = Path(__file__).resolve().parent
ARTIFACTS = Path("/home/fry/artifacts")

def sha(raw):
    return hashlib.sha256(raw).hexdigest()

def validate_witness(witness):
    """Reject missing/nonempty raw-write evidence and any measured effect."""
    if (witness.get("ordered_raw_writes") != [] or witness.get("nonboundary") is not True
            or any(type(witness.get(name)) is not int or witness[name] != 0 for name in
                   ("backend_put_attempts", "commit_attempts", "trie_mutation_attempts"))):
        raise RuntimeError("synthetic effect invariant failed")


def manifest_drift_cases(manifest):
    """Change/remove every field and add unknown keys at every object level."""
    def at(root, path):
        for key in path:
            root = root[key]
        return root

    def visit(node, path):
        changed = copy.deepcopy(manifest)
        replacement = False if node is None else None
        if path:
            at(changed, path[:-1])[path[-1]] = replacement
        else:
            changed = replacement
        yield "changed", path, changed
        if isinstance(node, dict):
            unknown = copy.deepcopy(manifest)
            at(unknown, path)["unknown_input"] = 0
            yield "unknown", path, unknown
            for key, value in node.items():
                missing = copy.deepcopy(manifest)
                del at(missing, path)[key]
                yield "missing", path + [key], missing
                yield from visit(value, path + [key])
        elif isinstance(node, list):
            extra = copy.deepcopy(manifest)
            at(extra, path).append(None)
            yield "extra", path, extra
            for index, value in enumerate(node):
                yield from visit(value, path + [index])
        else:
            # Also reject same-type value drift, including zero/default values.
            changed = copy.deepcopy(manifest)
            value = (not node if isinstance(node, bool) else node + 1 if isinstance(node, int)
                     else node + "drift" if isinstance(node, str) else 0)
            at(changed, path[:-1])[path[-1]] = value
            yield "value", path, changed
    yield from visit(manifest, [])


def check_manifest_binding(binary, fixture, tree):
    positive = subprocess.run([str(binary), str(fixture), "--validate-only"],
                              capture_output=True, text=True)
    if positive.returncode != 0:
        raise RuntimeError(positive.stderr)
    controls = []
    drift = tree / "drift-input.json"
    for mode, path, changed in manifest_drift_cases(json.loads(fixture.read_bytes())):
        drift.write_text(json.dumps(changed))
        result = subprocess.run([str(binary), str(drift), "--validate-only"],
                                capture_output=True, text=True)
        if result.returncode == 0 or "synthetic input contract drift" not in result.stderr:
            raise RuntimeError(f"manifest negative failed: {mode} {path}: {result.stderr}")
        controls.append({"mode": mode, "path": path, "exit_status": result.returncode,
                         "stdout": result.stdout, "stderr": result.stderr})
    return {"positive_accepted": True, "negative_count": len(controls), "controls": controls}


def run(output):
    # This runner only accepts a new artifact file under the persistent artifact
    # root. It never resolves, opens or searches protected database paths.
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    output = output.parent.resolve(strict=True) / output.name
    if not output.is_relative_to(ARTIFACTS.resolve()) or output.exists() or output.is_symlink():
        raise RuntimeError("output must be a new persistent artifact file")
    tree = Path(tempfile.mkdtemp(prefix="n4-synthetic-go-", dir=ARTIFACTS))
    archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", REVISION])
    (tree / "source.tar").write_bytes(archive)
    subprocess.run(["tar", "-x", "-C", str(tree)], input=archive, check=True)
    patch = patch_archive(tree)
    account_target = tree / TRACE_TARGET
    account_bytes = account_target.read_bytes()
    if sha(account_bytes) != TRACE_SHA256 or account_bytes.count(TRACE_NEEDLE) != 1:
        raise RuntimeError("raw-write observer source drift")
    account_target.write_bytes(account_bytes.replace(TRACE_NEEDLE, TRACE_INSERTION, 1))
    (account_target.parent / "synthetic_raw_write_observer.go").write_bytes(TRACE_HELPER)
    patch["raw_write_observer"] = {"target": str(TRACE_TARGET), "source_sha256": TRACE_SHA256, "patched_sha256": sha(account_target.read_bytes()), "helper_sha256": sha(TRACE_HELPER)}
    command = tree / "cmd/synthetic_native_transition_reference"
    command.mkdir(parents=True)
    harness = (HERE / "synthetic_native_transition_reference.go").read_bytes()
    (command / "main.go").write_bytes(harness)
    fixture_bytes = (HERE / "fixtures/synthetic_native_transition_input.json").read_bytes()
    fixture = tree / "synthetic-input.json"
    fixture.write_bytes(fixture_bytes)
    binary = tree / "synthetic-reference"
    subprocess.run(["go", "build", "-mod=readonly", "-o", str(binary), "./cmd/synthetic_native_transition_reference"], cwd=tree, check=True)
    binding = check_manifest_binding(binary, fixture, tree)
    result = subprocess.check_output([str(binary), str(fixture)], cwd=tree)
    witness = json.loads(result)
    validate_witness(witness)
    control_command = tree / "cmd/synthetic_native_observer_control"
    control_command.mkdir(parents=True)
    (control_command / "main.go").write_bytes((HERE / "empty_native_effects_reference.go").read_bytes())
    controls = [json.loads(subprocess.check_output(["go", "run", "-mod=readonly", "./cmd/synthetic_native_observer_control", "--control", mode], cwd=tree)) for mode in ("start", "delete")]
    if not all(item["rejected"] and item["mutation_attempts"] == 1 for item in controls):
        raise RuntimeError("observer controls failed")
    report = {"schema": 1, "manifest_binding": binding, "negative_controls": controls, "immutable_go_revision": REVISION, "immutable_source_sha256": SOURCE_PINS, "observer_archive_patch": patch,
              "archive_path": str(tree), "archive_sha256": sha(archive), "go_version": subprocess.check_output(["go", "version"]).decode().strip(),
              "control_harness_sha256": sha((HERE / "empty_native_effects_reference.go").read_bytes()), "fixture_sha256": sha(fixture_bytes), "harness_sha256": sha(harness), "runner_sha256": sha(Path(__file__).read_bytes()), "witness": witness}
    with output.open("x") as file:
        json.dump(report, file, indent=2)
        file.write("\n")
    print(str(output))

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    run(parser.parse_args().output)
