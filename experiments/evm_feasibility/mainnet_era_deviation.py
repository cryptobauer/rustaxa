#!/usr/bin/env python3
"""Fail-closed metadata admission for separately qualified semantic deviations."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat

_LIMIT = 1024 * 1024
_HEX = re.compile(r"[0-9a-f]{64}\Z")
_IDENTITY = ("deviation_id", "base_revision", "candidate", "source_sha256", "fixture_sha256")
_ACTIVATION = ("network", "config", "semantics_version", "boundary")


def _object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate JSON key")
        value[key] = item
    return value


def _read(path):
    path = Path(path)
    if not path.is_absolute() or path.suffix != ".json" or "AI.md" in path.parts or ".." in path.parts:
        raise ValueError("unsafe metadata path")
    for part in (path, *path.parents):
        if part.is_symlink():
            raise ValueError("symlink metadata path")
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode):
        raise ValueError("metadata must be a regular file")
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_size > _LIMIT:
            raise ValueError("metadata must be a bounded regular file")
        with os.fdopen(fd, "rb", closefd=False) as f:
            raw = f.read(_LIMIT + 1)
        if len(raw) > _LIMIT:
            raise ValueError("metadata exceeds limit")
    finally:
        os.close(fd)
    def invalid_constant(_value):
        raise ValueError("non-finite JSON number")
    doc = json.loads(raw, object_pairs_hook=_object, parse_constant=invalid_constant)
    if not isinstance(doc, dict):
        raise ValueError("metadata must be an object")
    return raw, doc


def _pin(pin):
    if not isinstance(pin, dict) or set(pin) != {"path", "sha256"}:
        raise ValueError("invalid evidence pin")
    digest = pin["sha256"]
    if not isinstance(digest, str) or not _HEX.fullmatch(digest):
        raise ValueError("invalid evidence hash")
    raw, document = _read(pin["path"])
    if hashlib.sha256(raw).hexdigest() != digest:
        raise ValueError("evidence hash mismatch")
    return document


def _facts(value, fields):
    if not isinstance(value, dict) or set(value) != set(fields):
        raise ValueError("invalid identity or activation fields")
    for name in fields:
        item = value[name]
        if name == "boundary":
            if not isinstance(item, int) or isinstance(item, bool) or item < 0:
                raise ValueError("invalid activation boundary")
        elif not isinstance(item, str) or not item or item != item.strip():
            raise ValueError("missing identity or activation field")
    return value


def validate_manifest(path: Path) -> dict:
    """Classify supplied metadata; admission is not protocol or parity approval."""
    try:
        _, doc = _read(path)
        if doc.get("schema_version") != 1 or type(doc.get("schema_version")) is not int:
            raise ValueError("unsupported schema version")
        keys = {"schema_version", "identity", "inputs", "activation", "classification", "corrected_namespace", "history", "evidence", "evidence_kind"}
        if set(doc) != keys:
            raise ValueError("unknown or missing manifest fields")
        evidence_kind = doc["evidence_kind"]
        if evidence_kind not in ("SYNTHETIC", "REAL"):
            raise ValueError("unknown evidence kind")
        identity = _facts(doc.get("identity"), _IDENTITY)
        for name in ("source_sha256", "fixture_sha256"):
            if not _HEX.fullmatch(identity[name]):
                raise ValueError("invalid source or fixture hash")
        if not (re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", identity["base_revision"]) and
                re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", identity["candidate"])):
            raise ValueError("invalid revision or candidate binding")
        inputs = doc.get("inputs")
        if not isinstance(inputs, dict) or set(inputs) != {"source", "fixture"}:
            raise ValueError("source and fixture metadata pins required")
        for name in ("source", "fixture"):
            _pin(inputs[name])
            if inputs[name]["sha256"] != identity[name + "_sha256"]:
                raise ValueError("source or fixture binding mismatch")
        activation = _facts(doc.get("activation"), _ACTIVATION)
        if doc.get("classification") != "CORRECTED_SEMANTICS":
            raise ValueError("corrected semantics classification required")
        namespace = doc.get("corrected_namespace")
        if not isinstance(namespace, str) or not namespace or namespace != namespace.strip() or namespace.lower() == "historical":
            raise ValueError("separate corrected fixture namespace required")
        history = doc.get("history")
        if not isinstance(history, dict) or set(history) != {"baseline", "current"}:
            raise ValueError("both historical corpus pins required")
        baseline, current = _pin(history["baseline"]), _pin(history["current"])
        if not isinstance(baseline.get("entries"), list) or not baseline["entries"] or baseline["entries"] != current.get("entries"):
            raise ValueError("historical entries or roots changed")
        for field in ("namespace", "semantics_version"):
            value = baseline.get(field)
            if not isinstance(value, str) or not value or value != value.strip() or current.get(field) != value:
                raise ValueError("historical namespace and version must match")
        head = baseline.get("head_period")
        if type(head) is not int or head < 0 or type(current.get("head_period")) is not int or current["head_period"] != head:
            raise ValueError("historical head period must match")
        if namespace == baseline["namespace"] or activation["semantics_version"] == baseline["semantics_version"]:
            raise ValueError("corrected identity collides with historical identity")
        if activation["boundary"] <= head:
            raise ValueError("activation must follow preserved history")
        for entry in baseline["entries"]:
            if not isinstance(entry, dict) or not isinstance(entry.get("id"), str) or not entry["id"] or entry["id"] != entry["id"].strip() or not isinstance(entry.get("root"), str) or not _HEX.fullmatch(entry["root"]):
                raise ValueError("historical entry identity and root required")
        if len({entry["id"] for entry in baseline["entries"]}) != len(baseline["entries"]):
            raise ValueError("duplicate historical entry")
        evidence = doc.get("evidence")
        expected = {"defect_review": "VERIFIED_PROTOCOL_RULE_VIOLATION", "normative_approval": "APPROVED_NORMATIVE_RULE", "semantic_approval": "APPROVED_INTENDED_BEHAVIOR", "activation_approval": "APPROVED_ACTIVATION"}
        if not isinstance(evidence, dict) or set(evidence) != set(expected):
            raise ValueError("complete evidence records required")
        records = {}
        for name, decision in expected.items():
            record = _pin(evidence[name])
            if type(record.get("schema_version")) is not int or record["schema_version"] != 1 or record.get("decision") != decision:
                raise ValueError("evidence decision not approved")
            if (record.get("evidence_kind") != evidence_kind
                    or record.get("identity") != identity or record.get("activation") != activation
                    or record.get("history_baseline_sha256") != history["baseline"]["sha256"]
                    or record.get("corrected_namespace") != namespace):
                raise ValueError("evidence binding mismatch")
            records[name] = record
        defect, rule = records["defect_review"], records["normative_approval"]
        if not isinstance(defect.get("actual_behavior"), str) or not defect["actual_behavior"].strip() or not isinstance(defect.get("normative_rule"), str) or not defect["normative_rule"].strip() or rule.get("normative_rule") != defect["normative_rule"]:
            raise ValueError("verified behavior and approved normative rule required")
        semantic = records["semantic_approval"]
        if not isinstance(semantic.get("invariants"), list) or not semantic["invariants"] or any(not isinstance(v, str) or not v.strip() for v in semantic["invariants"]) or not isinstance(semantic.get("expected_behavior"), str) or not semantic["expected_behavior"].strip():
            raise ValueError("approved invariants and intended behavior required")
        if records["activation_approval"].get("owner_approved") is not True:
            raise ValueError("owner activation approval required")
        return {"decision": "ADMITTED_FOR_SEPARATE_QUALIFICATION", "reasons": [], "deviation_id": identity["deviation_id"], "classification": "CORRECTED_SEMANTICS", "historical_parity": False, "blocked_items": "OPEN", "historical_failures": "OPEN", "qualification_scope": "METADATA_CONSISTENCY_ONLY", "protocol_correctness_accepted": False, "synthetic_evidence": evidence_kind == "SYNTHETIC", "authority_authenticated": False}
    except (OSError, ValueError, TypeError, KeyError, RecursionError) as exc:
        return {"decision": "NOT_ADMITTED", "reasons": [str(exc)], "qualification_scope": "METADATA_CONSISTENCY_ONLY", "protocol_correctness_accepted": False, "historical_parity": False}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    result = validate_manifest(args.manifest)
    out = args.output
    if not out.is_absolute() or "AI.md" in out.parts or ".." in out.parts or any(p.is_symlink() for p in (out, *out.parents)):
        raise ValueError("unsafe report path")
    fd = os.open(out, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(result, f, sort_keys=True, indent=2)
        f.write("\n")
        f.flush()
        os.fsync(f.fileno())
    parent = os.open(out.parent, os.O_RDONLY)
    try:
        os.fsync(parent)
    finally:
        os.close(parent)
    return 0 if result["decision"] == "ADMITTED_FOR_SEPARATE_QUALIFICATION" else 1


if __name__ == "__main__":
    raise SystemExit(main())
