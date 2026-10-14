"""Offline adversarial evidence-reference tests; no database access."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest

import evidence_records as evidence


class References(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir='/home/fry/artifacts', prefix='evidence-test-')
        self.root = Path(self.temp.name)
        self.body = self.root/'body'
        self.body.write_bytes(b'original complete output\n')
        self.contract_path = self.root/'contract'
        self.contract_path.write_bytes(b'fixed independently reviewed roster')
        self.contract = self.pin(self.contract_path)
        self.rows = [dict(role='full-failure', **self.pin(self.body))]
        self.destination = str(self.root/'accepted-review.md')
        self.raw = evidence.build(self.destination, self.contract, 'review remains separate', self.rows)

    def tearDown(self):
        self.temp.cleanup()

    @staticmethod
    def pin(path):
        raw = path.read_bytes()
        return dict(path=str(path), bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest())

    def verify(self, raw=None, rows=None):
        return evidence.verify(evidence.decode(self.raw if raw is None else raw), self.destination,
                               self.contract, self.rows if rows is None else rows)

    def test_complete_body_and_contract_charged(self):
        result = self.verify()
        self.assertEqual(result['reference_count'], 2)
        self.assertEqual(result['referenced_bytes'], len(self.body.read_bytes()) + self.contract['bytes'])
        self.assertEqual(result['streamed_bytes'], result['referenced_bytes'])

    def test_repeat_keeps_order_and_charge(self):
        rows = [self.rows[0], dict(self.rows[0], role='second-placement')]
        raw = evidence.build(self.destination, self.contract, '', rows)
        result = self.verify(raw, rows)
        self.assertEqual(result['reference_count'], 3)
        self.assertEqual(result['referenced_bytes'], 2*self.rows[0]['bytes']+self.contract['bytes'])
        value = evidence.decode(raw)
        value['references'].reverse()
        with self.assertRaises(ValueError):
            self.verify(json.dumps(value).encode(), rows)

    def test_same_size_drift(self):
        self.body.write_bytes(b'X'*self.rows[0]['bytes'])
        with self.assertRaises(ValueError):
            self.verify()

    def test_missing_complete_body(self):
        self.body.unlink()
        with self.assertRaises(FileNotFoundError):
            self.verify()

    def test_wrong_role_path_missing_extra_and_identity(self):
        for change in ('role', 'path', 'missing', 'extra', 'destination', 'ledger', 'accepted', 'contract'):
            value = evidence.decode(self.raw)
            if change in ('role', 'path'):
                value['references'][0][change] = 'wrong'
            elif change == 'missing':
                value['references'] = []
            elif change == 'extra':
                value['references'].append(self.rows[0])
            elif change == 'contract':
                value['contract']['sha256'] = '0'*64
            else:
                value[change] = True if change == 'accepted' else 'wrong'
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.verify(json.dumps(value).encode())

    def test_conflicting_repeat(self):
        rows = [self.rows[0], dict(self.rows[0], sha256='0'*64)]
        with self.assertRaises(ValueError):
            evidence.build(self.destination, self.contract, '', rows)

    def test_duplicate_key_and_extra_field(self):
        with self.assertRaises(ValueError):
            evidence.decode(b'{"a":1,"a":2}')
        value = evidence.decode(self.raw)
        value['unused'] = 1
        with self.assertRaises(ValueError):
            self.verify(json.dumps(value).encode())

    def test_count_exact_and_one_over(self):
        rows = [dict(self.rows[0], role=str(i)) for i in range(31)]
        self.assertEqual(self.verify(evidence.build(self.destination, self.contract, '', rows), rows)['reference_count'], 32)
        with self.assertRaises(ValueError):
            evidence.build(self.destination, self.contract, '', rows+[dict(self.rows[0], role='overflow')])

    def test_total_exact_and_one_over(self):
        self.body.write_bytes(b'x'*(evidence.TOTAL_CAP-self.contract['bytes']))
        rows = [dict(role='body', **self.pin(self.body))]
        self.assertEqual(self.verify(evidence.build(self.destination, self.contract, '', rows), rows)['referenced_bytes'], evidence.TOTAL_CAP)
        self.body.write_bytes(self.body.read_bytes()+b'x')
        rows = [dict(role='body', **self.pin(self.body))]
        with self.assertRaises(ValueError):
            evidence.build(self.destination, self.contract, '', rows)

    def test_body_one_over_and_boolean_bytes(self):
        for size in (evidence.BODY_CAP+1, True, -1):
            with self.assertRaises(ValueError):
                evidence.check_row(dict(self.rows[0], bytes=size))

    def test_input_summary_node_depth_caps(self):
        with self.assertRaises(ValueError):
            evidence.decode(b' '* (evidence.WRAPPER_CAP+1))
        with self.assertRaises(ValueError):
            evidence.build(self.destination, self.contract, 'x'*(evidence.SUMMARY_CAP+1), self.rows)
        with self.assertRaises(ValueError):
            evidence.bounded_shape([0]*evidence.NODE_CAP)
        value = 0
        for _ in range(evidence.DEPTH_CAP+1):
            value = [value]
        with self.assertRaises(ValueError):
            evidence.bounded_shape(value)

    def test_lexical_forbidden_and_symlink_paths(self):
        bad = ['/a//b', '/a/./b', '/a/../b', '/a\\b', '/a\nb', 'relative', '/AI.md',
               str(evidence.REPO/'local/evm-state-db/db'), str(evidence.REPO/'data/db')]
        for path in bad:
            with self.subTest(path=path), self.assertRaises(ValueError):
                evidence.canonical_path(path)
        alias = self.root/'alias'
        alias.symlink_to(self.body)
        with self.assertRaises(ValueError):
            evidence.authenticate(dict(self.rows[0], path=str(alias)))

    def test_inode_alias(self):
        alias = self.root/'hardlink'
        os.link(self.body, alias)
        rows = [self.rows[0], dict(self.rows[0], path=str(alias), role='alias')]
        with self.assertRaises(ValueError):
            evidence.build(self.destination, self.contract, '', rows)

    def test_output_collision_and_no_prefix(self):
        evidence.publish(self.destination, self.raw, self.destination, self.contract, self.rows)
        with self.assertRaises(FileExistsError):
            evidence.publish(self.destination, self.raw, self.destination, self.contract, self.rows)
        self.assertEqual(Path(self.destination).read_bytes(), self.raw)
        with self.assertRaises(FileExistsError):
            evidence.publish(self.body, self.raw, self.destination, self.contract, self.rows)
        output = self.root/'should-not-exist'
        self.body.write_bytes(b'drift')
        with self.assertRaises(ValueError):
            evidence.publish(output, self.raw, self.destination, self.contract, self.rows)
        self.assertFalse(output.exists())

    def test_contract_drift(self):
        self.contract_path.write_bytes(b'X'*self.contract['bytes'])
        with self.assertRaises(ValueError):
            self.verify()


if __name__ == '__main__':
    unittest.main()
