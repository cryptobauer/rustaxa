"""Synthetic unit tests for the Magnolia deviation admission boundary.

These records exercise schema and cross-binding only. They are not Go evidence.
"""
import hashlib
import json
import os
import tempfile
import unittest
from unittest import mock
from pathlib import Path

import mainnet_era_deviation as deviation
main = deviation.main
validate_manifest = deviation.validate_manifest


def dump(path: Path, value: dict) -> str:
    raw = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(raw)
    return hashlib.sha256(raw).hexdigest()


def pin(path: Path) -> dict:
    return {"path": str(path.resolve()), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


class MagnoliaDeviationAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="magnolia-deviation-synthetic-")
        self.root = Path(self.tmp.name)
        self.manifest_path, self.manifest = self.make_case()

    def tearDown(self):
        self.tmp.cleanup()

    def make_case(self):
        root = self.root
        identity = {
            "deviation_id": "synthetic-magnolia-case",
            "base_revision": "a" * 40,
            "candidate": "b" * 40,
            "source_sha256": "",
            "fixture_sha256": "",
        }
        activation = {
            "network": "synthetic",
            "config": "synthetic-config",
            "semantics_version": "corrected-v1",
            "boundary": 101,
        }
        source_path = root / "source-metadata.json"
        fixture_path = root / "fixture-metadata.json"
        source_hash = dump(source_path, {"kind": "synthetic-source-metadata", "root": "c" * 64})
        fixture_hash = dump(fixture_path, {"kind": "synthetic-fixture-metadata", "root": "d" * 64})
        identity["source_sha256"] = source_hash
        identity["fixture_sha256"] = fixture_hash
        history_path = root / "history.json"
        history = {
            "namespace": "legacy-fixture",
            "semantics_version": "legacy-v1",
            "head_period": 100,
            "entries": [{"id": "synthetic-period-1", "root": "e" * 64}],
        }
        history_hash = dump(history_path, history)
        baseline = {"path": str(history_path.resolve()), "sha256": history_hash}
        current_path = root / "current-history.json"
        current_hash = dump(current_path, history)
        current = {"path": str(current_path.resolve()), "sha256": current_hash}
        corrected_namespace = "corrected-rules-v1"
        common = {
            "schema_version": 1,
            "identity": identity,
            "activation": activation,
            "history_baseline_sha256": baseline["sha256"],
            "corrected_namespace": corrected_namespace,
            "evidence_kind": "SYNTHETIC",
        }
        evidence_values = {
            "defect_review": {
                **common,
                "decision": "VERIFIED_PROTOCOL_RULE_VIOLATION",
                "actual_behavior": "synthetic observed behavior A",
                "normative_rule": "synthetic normative invariant A",
            },
            "normative_approval": {
                **common,
                "decision": "APPROVED_NORMATIVE_RULE",
                "normative_rule": "synthetic normative invariant A",
            },
            "semantic_approval": {
                **common,
                "decision": "APPROVED_INTENDED_BEHAVIOR",
                "invariants": ["synthetic invariant A"],
                "expected_behavior": "synthetic corrected behavior B",
            },
            "activation_approval": {
                **common,
                "decision": "APPROVED_ACTIVATION",
                "owner_approved": True,
            },
        }
        evidence = {}
        for name, value in evidence_values.items():
            path = root / (name + ".json")
            dump(path, value)
            evidence[name] = pin(path)
        manifest = {
            "schema_version": 1,
            "identity": identity,
            "activation": activation,
            "classification": "CORRECTED_SEMANTICS",
            "evidence_kind": "SYNTHETIC",
            "corrected_namespace": corrected_namespace,
            "inputs": {"source": pin(source_path), "fixture": pin(fixture_path)},
            "history": {"baseline": baseline, "current": current},
            "evidence": evidence,
        }
        manifest_path = root / "manifest.json"
        dump(manifest_path, manifest)
        return manifest_path, manifest

    def save(self):
        dump(self.manifest_path, self.manifest)

    def repin_evidence(self):
        for name, item in self.manifest["evidence"].items():
            path = Path(item["path"])
            doc = json.loads(path.read_text())
            doc["identity"] = self.manifest["identity"]
            doc["activation"] = self.manifest["activation"]
            doc["corrected_namespace"] = self.manifest["corrected_namespace"]
            doc["history_baseline_sha256"] = self.manifest["history"]["baseline"]["sha256"]
            doc["evidence_kind"] = self.manifest["evidence_kind"]
            item["sha256"] = dump(path, doc)

    def rewrite_source(self, raw: bytes, suffix: str = ".json"):
        old = Path(self.manifest["inputs"]["source"]["path"])
        path = old.with_suffix(suffix)
        if old != path and old.exists():
            old.unlink()
        path.write_bytes(raw)
        digest = hashlib.sha256(raw).hexdigest()
        self.manifest["inputs"]["source"] = {"path": str(path.resolve()), "sha256": digest}
        self.manifest["identity"]["source_sha256"] = digest
        self.repin_evidence()
        self.save()
        return path

    def reject(self):
        result = validate_manifest(self.manifest_path)
        self.assertEqual(result.get("decision"), "NOT_ADMITTED", result)
        self.assertIsInstance(result.get("reasons"), list)
        self.assertTrue(result["reasons"])

    def test_coherent_synthetic_evidence_is_only_admitted_for_separate_qualification(self):
        result = validate_manifest(self.manifest_path)
        self.assertEqual(result.get("decision"), "ADMITTED_FOR_SEPARATE_QUALIFICATION", result)
        self.assertIsInstance(result.get("reasons"), list)
        self.assertNotEqual(result["decision"], "PASS")
        self.assertTrue(result.get("synthetic_evidence"))
        self.assertEqual(result.get("qualification_scope"), "METADATA_CONSISTENCY_ONLY")
        self.assertIs(result.get("protocol_correctness_accepted"), False)
        self.assertIs(result.get("historical_parity"), False)
        self.assertIs(result.get("authority_authenticated"), False)

    def test_evidence_kind_manifest_mismatch_and_missing_record_kind_are_rejected(self):
        self.manifest["evidence_kind"] = "REAL"
        self.save()
        self.reject()

        self.manifest_path, self.manifest = self.make_case()
        item = self.manifest["evidence"]["semantic_approval"]
        path = Path(item["path"])
        record = json.loads(path.read_text())
        record["evidence_kind"] = "REAL"
        item["sha256"] = dump(path, record)
        self.save()
        self.reject()

        self.manifest_path, self.manifest = self.make_case()
        item = self.manifest["evidence"]["activation_approval"]
        path = Path(item["path"])
        record = json.loads(path.read_text())
        del record["evidence_kind"]
        item["sha256"] = dump(path, record)
        self.save()
        self.reject()

    def test_provider_warning_alone_is_not_a_verified_protocol_violation(self):
        doc = json.loads(Path(self.manifest["evidence"]["defect_review"]["path"]).read_text())
        doc["decision"] = "PROVIDER_WARNING"
        digest = dump(Path(self.manifest["evidence"]["defect_review"]["path"]), doc)
        self.manifest["evidence"]["defect_review"]["sha256"] = digest
        self.save()
        self.reject()

    def test_mismatch_metadata_not_defect_evidence(self):
        path = Path(self.manifest["evidence"]["defect_review"]["path"])
        index_shaped_metadata = {
            "schema": 1,
            "status": "EVIDENCE_INDEX_ONLY_NO_CAUSE_DIAGNOSIS",
            "scope": "Synthetic evidence index metadata; no verified defect claim.",
            "classification_limit": "A test mismatch and provider warning do not establish a protocol violation.",
        }
        digest = dump(path, index_shaped_metadata)
        self.manifest["evidence"]["defect_review"]["sha256"] = digest
        self.save()
        self.reject()

    def test_mismatch_only_is_not_a_verified_protocol_violation(self):
        doc = json.loads(Path(self.manifest["evidence"]["defect_review"]["path"]).read_text())
        doc["decision"] = "OBSERVED_MISMATCH"
        digest = dump(Path(self.manifest["evidence"]["defect_review"]["path"]), doc)
        self.manifest["evidence"]["defect_review"]["sha256"] = digest
        self.save()
        self.reject()

    def test_missing_stale_and_conflicting_approvals_are_rejected(self):
        del self.manifest["evidence"]["activation_approval"]
        self.save()
        self.reject()

        self.manifest_path, self.manifest = self.make_case()
        approval_path = Path(self.manifest["evidence"]["semantic_approval"]["path"])
        approval = json.loads(approval_path.read_text())
        approval["identity"]["candidate"] = "f" * 40
        digest = dump(approval_path, approval)
        self.manifest["evidence"]["semantic_approval"]["sha256"] = digest
        self.save()
        self.reject()

        self.manifest_path, self.manifest = self.make_case()
        approval_path = Path(self.manifest["evidence"]["normative_approval"]["path"])
        approval = json.loads(approval_path.read_text())
        approval["normative_rule"] = "conflicting synthetic invariant"
        digest = dump(approval_path, approval)
        self.manifest["evidence"]["normative_approval"]["sha256"] = digest
        self.save()
        self.reject()

    def test_candidate_activation_and_stale_input_hashes_are_rejected(self):
        self.manifest["identity"]["candidate"] = "f" * 40
        self.save()
        self.reject()

        self.manifest_path, self.manifest = self.make_case()
        self.manifest["activation"]["boundary"] = 124
        self.save()
        self.reject()

        self.manifest_path, self.manifest = self.make_case()
        self.manifest["identity"]["fixture_sha256"] = "0" * 64
        self.save()
        self.reject()

    def test_history_root_replacement_and_fixture_replacement_are_rejected(self):
        path = Path(self.manifest["history"]["current"]["path"])
        changed = json.loads(path.read_text())
        changed["entries"][0]["root"] = "f" * 64
        digest = dump(path, changed)
        self.manifest["history"]["current"]["sha256"] = digest
        self.save()
        self.reject()

        self.manifest_path, self.manifest = self.make_case()
        fixture = Path(self.manifest["inputs"]["fixture"]["path"])
        dump(fixture, {"kind": "replacement-of-synthetic-fixture", "root": "f" * 64})
        self.reject()

    def test_corrected_semantics_cannot_be_relabelled_as_historical_parity(self):
        self.manifest["classification"] = "HISTORICAL_PARITY"
        self.save()
        self.reject()

    def test_unknown_schema_decision_and_kind_are_rejected(self):
        self.manifest["schema_version"] = 99
        self.save()
        self.reject()

        self.manifest_path, self.manifest = self.make_case()
        doc_path = Path(self.manifest["evidence"]["activation_approval"]["path"])
        doc = json.loads(doc_path.read_text())
        doc["decision"] = "UNKNOWN_APPROVAL"
        digest = dump(doc_path, doc)
        self.manifest["evidence"]["activation_approval"]["sha256"] = digest
        self.save()
        self.reject()

        self.manifest_path, self.manifest = self.make_case()
        self.manifest["activation"]["semantics_version"] = "unknown-version"
        self.repin_evidence()
        self.save()

        self.manifest_path, self.manifest = self.make_case()
        self.manifest["evidence_kind"] = "UNKNOWN_KIND"
        self.save()
        self.reject()

    def test_namespace_and_activation_version_collisions_are_rejected(self):
        self.manifest["corrected_namespace"] = "legacy-fixture"
        self.repin_evidence()
        self.save()
        self.reject()

        self.manifest_path, self.manifest = self.make_case()
        self.manifest["activation"]["semantics_version"] = "legacy-v1"
        self.repin_evidence()
        self.save()
        self.reject()

    def test_activation_boundary_must_follow_both_history_heads(self):
        self.manifest["activation"]["boundary"] = 100
        self.repin_evidence()
        self.save()
        self.reject()

    def test_missing_history_fields_are_rejected(self):
        for key in ("namespace", "semantics_version", "head_period"):
            self.manifest_path, self.manifest = self.make_case()
            for side in ("baseline", "current"):
                path = Path(self.manifest["history"][side]["path"])
                data = json.loads(path.read_text())
                del data[key]
                self.manifest["history"][side]["sha256"] = dump(path, data)
            self.repin_evidence()
            self.save()
            self.reject()

    def test_non_json_metadata_is_rejected_without_reading_non_json_suffix(self):
        valid_json = b'{"kind":"synthetic-source-metadata","root":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}\n'
        source = self.rewrite_source(valid_json, suffix=".txt")
        original_open = deviation.os.open
        opened = []
        def guarded_open(path, flags, *args, **kwargs):
            candidate = Path(os.fsdecode(path))
            if candidate == source:
                opened.append(candidate)
                raise AssertionError("non-JSON input body was opened")
            return original_open(path, flags, *args, **kwargs)
        with mock.patch.object(deviation.os, "open", guarded_open):
            self.reject()
        self.assertEqual(opened, [])

        self.manifest_path, self.manifest = self.make_case()
        self.rewrite_source(b"not JSON\n", suffix=".json")
        self.reject()

    def test_non_finite_json_numbers_are_rejected(self):
        self.rewrite_source(b'{"kind":"synthetic-source-metadata","value":NaN}\n')
        self.reject()
        self.manifest_path, self.manifest = self.make_case()
        self.rewrite_source(b'{"kind":"synthetic-source-metadata","value":Infinity}\n')
        self.reject()

    @unittest.skipUnless(hasattr(os, "mkfifo"), "requires POSIX FIFO support")
    def test_fifo_metadata_input_is_rejected_without_blocking(self):
        source = Path(self.manifest["inputs"]["source"]["path"])
        source.unlink()
        fifo = self.root / "source-fifo.json"
        os.mkfifo(fifo)
        self.manifest["inputs"]["source"] = {"path": str(fifo), "sha256": "0" * 64}
        self.save()
        original_open = deviation.os.open
        def guarded_open(path, flags, *args, **kwargs):
            if Path(os.fsdecode(path)) == fifo:
                if not flags & os.O_NONBLOCK:
                    raise AssertionError("FIFO must be opened with O_NONBLOCK")
            return original_open(path, flags, *args, **kwargs)
        with mock.patch.object(deviation.os, "open", guarded_open):
            self.reject()

    def test_unknown_manifest_key_is_rejected(self):
        self.manifest["unrecognized_field"] = "must not be silently ignored"
        self.save()
        self.reject()

    def test_symlink_manifest_and_symlink_input_are_rejected(self):
        link = self.root / "manifest-link.json"
        link.symlink_to(self.manifest_path)
        result = validate_manifest(link)
        self.assertEqual(result.get("decision"), "NOT_ADMITTED", result)

        self.manifest_path, self.manifest = self.make_case()
        source = Path(self.manifest["inputs"]["source"]["path"])
        linked = self.root / "source-link.json"
        linked.symlink_to(source)
        self.manifest["inputs"]["source"] = {
            "path": str(linked),
            "sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        }
        self.save()
        self.reject()

    def test_rejected_cli_writes_immutable_rejection_and_preserves_existing_output(self):
        self.manifest["classification"] = "HISTORICAL_PARITY"
        self.save()
        absent = self.root / "new-output.json"
        code = main([str(self.manifest_path), "--output", str(absent)])
        self.assertEqual(code, 1)
        report = json.loads(absent.read_text())
        self.assertEqual(report.get("decision"), "NOT_ADMITTED")
        self.assertNotEqual(report.get("decision"), "PASS")
        self.assertIs(report.get("protocol_correctness_accepted"), False)
        self.assertIs(report.get("historical_parity"), False)

        existing = self.root / "existing-output.json"
        sentinel = b"keep-existing-bytes\n"
        existing.write_bytes(sentinel)
        with self.assertRaises(FileExistsError):
            main([str(self.manifest_path), "--output", str(existing)])
        self.assertEqual(existing.read_bytes(), sentinel)


if __name__ == "__main__":
    unittest.main()
