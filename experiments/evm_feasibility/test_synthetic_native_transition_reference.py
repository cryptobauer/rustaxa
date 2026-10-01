"""Focused fail-closed controls for the measured lifecycle witness."""
import copy
import unittest
from synthetic_native_transition_reference import validate_witness


class WitnessControls(unittest.TestCase):
    def test_raw_writes_and_incomplete_witness_fail_closed(self):
        valid = {"nonboundary": True, "ordered_raw_writes": [],
                 "backend_put_attempts": 0, "commit_attempts": 0,
                 "trie_mutation_attempts": 0}
        validate_witness(valid)
        for value in ([{"address": "fe", "key": "04", "value": "00"}], None, {}, 0):
            changed = copy.deepcopy(valid)
            changed["ordered_raw_writes"] = value
            with self.assertRaises(RuntimeError):
                validate_witness(changed)
        for field in valid:
            changed = copy.deepcopy(valid)
            del changed[field]
            with self.assertRaises(RuntimeError):
                validate_witness(changed)
        for field in ("backend_put_attempts", "commit_attempts", "trie_mutation_attempts"):
            changed = copy.deepcopy(valid)
            changed[field] = 1
            with self.assertRaises(RuntimeError):
                validate_witness(changed)


if __name__ == "__main__":
    unittest.main()
