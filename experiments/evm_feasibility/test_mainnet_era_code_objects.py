"""Finite manual schema1 fixtures; no execution/physical data is synthesized by decoding."""
import copy
import json
import unittest

from mainnet_era_code_objects import (CodeObjectError, code_objects_binding,
                                     decode_code_objects, strict_json_loads)
from mainnet_era_compare import keccak


def account(body='6000', tag='a'):
    raw=bytes.fromhex(body)
    return {'address':tag,'present':True,'code':body,'code_hash':keccak(raw).hex(),
            'code_size':len(raw),'nonce':'0','raw_account':'literal unchanged bytes'}


def fixture():
    a=account();b=account('60016000','b')
    empty={'present':True,'code':'','code_hash':'','code_size':0}
    absent={'present':False}
    known=[]
    def accounts(*values):
        rows=list(values)
        known.extend(rows)
        return rows
    tx={'observer':{'catalog':{'accounts':accounts(a,copy.deepcopy(b))}},'input':'6000','ordered_raw_writes':[]}
    # These real schema locations intentionally share maps/slices before JSON
    # serialization, as the Go facts/periods do. Occurrences must not disappear.
    final={'accounts':accounts(copy.deepcopy(a),empty,absent),
           'native_catalog':{'accounts':accounts(copy.deepcopy(b))}}
    period={'transactions':[tx],'reward_input':{'transactions':[tx]},
            'distributions':[{'input':{'transactions':[tx]}}],'final':final}
    known.extend(tx['observer']['catalog']['accounts'] * 3) # reward,distribution,pending occurrences
    call={'before':{'accounts':accounts(copy.deepcopy(a))},
          'after':{'accounts':accounts(copy.deepcopy(b))},'ordered_raw_writes':[{'literal':'do not transform'}]}
    known.extend(call['before']['accounts'] * 2)
    known.extend(call['after']['accounts'] * 2)
    donor={'periods':[{'final':{'accounts':accounts(copy.deepcopy(b))}}]}
    raw={'genesis':{'native_catalog':{'accounts':accounts(copy.deepcopy(a))}},
         'seed':{'native_catalog':{'accounts':accounts(copy.deepcopy(a))},
                 'declared_projection_catalog':{'accounts':accounts(copy.deepcopy(a))},
                 'declared_donor_catalog':{'accounts':accounts(copy.deepcopy(b))},
                 'donor_document':donor},
         'periods':[period],'pending_reward_facts':[{'transactions':[tx]}],
         'pre_reward':{'accounts':accounts(copy.deepcopy(b)),
                       'native_catalog':{'accounts':accounts(copy.deepcopy(a))}},
         'api_cases':[{'trace':{'calls':[call]},'trace_sequence':{'calls':[call,call]}}],
         'prefix_periods':[{'final':{'accounts':[account('fe','unemitted prefix')]}}],
         'history_rows':[{'code':'physical code bytes, untouched','code_object':'physical lookalike'}],
         'rows':[{'code':'physical rows untouched'}], 'latest_rows':[{'code':'latest untouched'}],
         'unrelated':{'code':'not even hex','code_hash':'arbitrary','code_size':'arbitrary'},
         'logs':[{'data':'6000'}]}
    return raw,known


def manual_pack(raw,known):
    # Independent declared list of fixture paths, not the decoder's traversal.
    # First collect references, then mutate, so aliases cannot hide a bad code
    # observation or cause the second visit to lose its reference.
    encoded,accounts=copy.deepcopy((raw,known))
    pool={};replacements=[]
    for row in accounts:
        if row.get('code'):
            pool[row['code_hash']]=row['code']
            replacements.append((row,row['code_hash']))
    for row,key in replacements:
        row.pop('code',None)
        row['code_object']=key
    encoded['code_objects']=pool
    encoded.update(code_objects_binding(pool))
    return encoded


def rebind(encoded):
    encoded.update(code_objects_binding(encoded['code_objects']))
    return encoded


class CodeObjectTests(unittest.TestCase):
    def reject(self,document,**kwargs):
        with self.assertRaises(CodeObjectError):
            decode_code_objects(document,**kwargs)

    def test_complete_paths_aliases_and_json_roundtrip(self):
        raw,known=fixture()
        encoded=manual_pack(raw,known)
        before=copy.deepcopy(encoded)
        self.assertEqual(len(encoded['code_objects']),2)
        self.assertEqual(decode_code_objects(encoded),raw)
        self.assertEqual(encoded,before)
        serialized=json.dumps(encoded,separators=(',',':'))
        self.assertGreater(serialized.count('code_object"'),15)
        parsed=strict_json_loads(serialized)
        self.assertEqual(decode_code_objects(parsed),json.loads(json.dumps(raw)))
        self.assertIn('code',encoded['prefix_periods'][0]['final']['accounts'][0])
        self.assertEqual(encoded['history_rows'],raw['history_rows'])

    def test_missing_object_is_not_recovered_from_physical_history(self):
        raw,known=fixture();encoded=manual_pack(raw,known)
        key=next(iter(encoded['code_objects']));body=encoded['code_objects'].pop(key)
        encoded['history_rows'].append({key:body})
        self.reject(rebind(encoded))

    def test_wrong_code_hash_and_size(self):
        raw,known=fixture()
        for field,value in [('code_hash','00'*32),('code_size',99),('code_size',True),('code_object','ff'*32)]:
            encoded=manual_pack(raw,known)
            encoded['genesis']['native_catalog']['accounts'][0][field]=value
            self.reject(encoded)

    def test_altered_bytes_and_malformed_hex(self):
        raw,known=fixture()
        for value in ['ff','GG','0','600A',{'code_object':'cycle'}]:
            encoded=manual_pack(raw,known);key=next(iter(encoded['code_objects']))
            encoded['code_objects'][key]=value
            self.reject(rebind(encoded))

    def test_conflicting_raw_and_reference_fields(self):
        raw,known=fixture();encoded=manual_pack(raw,known)
        encoded['genesis']['native_catalog']['accounts'][0]['code']=''
        self.reject(encoded)

    def test_extra_and_empty_pool_objects(self):
        raw,known=fixture()
        for text in ['fe','']:
            encoded=manual_pack(raw,known)
            encoded['code_objects'][keccak(bytes.fromhex(text)).hex()]=text
            self.reject(rebind(encoded))

    def test_metadata_count_digest_schema_and_envelope(self):
        raw,known=fixture();original=manual_pack(raw,known)
        for field,value in [('code_objects_count',99),('code_objects_count',True),
                            ('code_objects_sha256','00'*32),('code_objects_schema',2),('code_objects_schema',True)]:
            encoded=copy.deepcopy(original);encoded[field]=value
            self.reject(encoded)
        for field in ['code_objects','code_objects_count','code_objects_sha256','code_objects_schema']:
            encoded=copy.deepcopy(original);del encoded[field]
            self.reject(encoded)
        binding=code_objects_binding(original['code_objects'])
        self.assertEqual(decode_code_objects(original,expected_binding=binding),raw)
        binding['code_objects_sha256']='00'*32
        self.reject(original,expected_binding=binding)

    def test_legacy_unchanged_and_rogue_reference_rejected(self):
        raw,_=fixture()
        self.assertEqual(decode_code_objects(raw),raw)
        changed=copy.deepcopy(raw)
        row=changed['genesis']['native_catalog']['accounts'][0]
        row['code_object']=row.pop('code_hash');row.pop('code')
        self.reject(changed)

    def test_empty_and_absent_code_distinctions_survive(self):
        raw={'periods':[{'final':{'accounts':[{'code':'','code_hash':'','code_size':0},{'present':False},{}]}}],
             'history_rows':[{'code':'unknown arbitrary physical value'}]}
        encoded=copy.deepcopy(raw);encoded['code_objects']={};encoded.update(code_objects_binding({}))
        self.assertEqual(decode_code_objects(encoded),raw)

    def test_callback_and_account_order_remain_visible(self):
        raw,known=fixture();encoded=manual_pack(raw,known)
        encoded['periods'][0]['final']['accounts'].reverse()
        encoded['api_cases'][0]['trace']['calls'][0]['stage']='changed callback'
        decoded=decode_code_objects(encoded)
        self.assertNotEqual(decoded,raw)
        self.assertEqual(decoded['periods'][0]['final']['accounts'],list(reversed(raw['periods'][0]['final']['accounts'])))

    def test_physical_changes_are_preserved_not_normalized(self):
        raw,known=fixture();encoded=manual_pack(raw,known)
        encoded['history_rows'][0]['code']='changed physical evidence'
        decoded=decode_code_objects(encoded)
        self.assertEqual(decoded['history_rows'][0]['code'],'changed physical evidence')
        self.assertNotEqual(decoded,raw)

    def test_nested_codec_and_cyclic_donor_rejected(self):
        raw,known=fixture();encoded=manual_pack(raw,known)
        encoded['seed']['donor_document']['code_objects']={}
        self.reject(encoded)
        cyclic={'seed':{}};cyclic['seed']['donor_document']=cyclic
        self.reject(cyclic)

    def test_duplicate_json_pool_key_rejected_before_decode(self):
        raw='{"code_objects":{"'+'11'*32+'":"00","'+'11'*32+'":"01"}}'
        with self.assertRaises(CodeObjectError):
            strict_json_loads(raw)


if __name__=='__main__':
    unittest.main()
