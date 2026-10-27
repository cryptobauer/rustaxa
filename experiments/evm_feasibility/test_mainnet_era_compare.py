"""Declared finite fixtures for the reviewed reward comparison contract.

No reference output is copied into the implementation. Small fixed/dynamic
arithmetic below is explicit and independently reviewable. The lead runs tests.
"""
import copy
import hashlib
import json
import unittest

from mainnet_era_compare import (ComparisonError, DPOS, comparison, comparison_digest,
                                compact, key, keccak, reward_key, rlp_encode, validator_key, flatten_donor)

A='11'*20
B='22'*20
C='33'*20


def put(k,value):
    return {'address':DPOS,'key':k,'value':value.hex()}


def pool(address,reward,commission):
    return put(reward_key(address),rlp_encode([reward,commission]))


def fixture(era='fixed'):
    # Fixed: reward20, DAG10, votes9, author1, map A11/B8/C0.
    # Dynamic: supply1000, maximum2000, yield1e6, reward100,
    # DAG50, votes45, author5, map A55/B40/C0.
    period={'fixed':2,'minted':12,'dynamic':22}[era]
    records=[put(validator_key(a),rlp_encode([[stake,commission,0,0],0]))
             for a,stake,commission in [(A,6000,1000),(B,4000,2000),(C,0,0)]]
    seed=records+[pool(a,0,0) for a in [A,B,C]]+[put(key(5),compact(10000))]
    if era=='minted':
        seed.append(put(key(6),compact(7)))
    if era=='dynamic':
        seed.append(put(key(7),compact(1000)))
    stats={'number':period,'block_author':A,'blocks_per_year':100,
           'max_votes_weight':3,'total_votes_weight':3,'total_dag_blocks_count':2,
           'validators':[{'validator':a,'dag_blocks_count':dag,'vote_weight':vote,
                          'fees_reward':'0','reward_key':reward_key(a)}
                         for a,dag,vote in [(A,1,2),(B,1,1),(C,0,0)]]}
    if era=='dynamic':
        writes=[put(key(8),rlp_encode(1000000)),pool(A,5,0),pool(A,55,5),pool(B,32,8),pool(C,0,0),put(key(7),compact(1100))]
        minted=100
    else:
        writes=[pool(A,1,0),pool(A,11,1),pool(B,7,1),pool(C,0,0)]
        if era=='minted':
            writes.append(put(key(6),compact(27)))
        minted=20
    part={'period':period,'input':stats,'minted_reward':str(minted),'ordered_raw_writes':writes}
    row={'number':period,'transactions':[],'distribution_order':[period],
         'ordered_raw_writes':{'begin_block':[],'rewards':copy.deepcopy(writes),'end_block':[]},
         'distributions':[part],'reward_output':{'minted_reward':str(minted)}}
    return {'schema':1,'configuration':{'dpos':{'blocks_per_year':100,'yield_percentage':20,
                'dag_proposers_reward_percent':50,'max_block_author_reward_percent':5},
                'hardforks':{'aspen':{'part_one_block':10,'part_two_block':20,
                    'max_supply':'2000','generated_rewards':'0'},'cacti':{'block':30},
                    'fix_redelegate_block':100,'redelegations':[]}},
            'inputs':{'initial_validators':[{'address':a} for a in [A,B,C]],
                      'genesis_allocations':[{'address':A,'balance':'1000'}]},
            'seed':{'period':period-1,'ordered_raw_writes':seed,
                    'native_catalog':{'slots':[{'address':r['address'],'key':r['key'],
                                               'value':r['value'],'present':True} for r in seed]}},
            'periods':[row],'history_rows':[{'unchanged':'physical bytes'}]}


def sync(document):
    for row in document['periods']:
        row['ordered_raw_writes']['rewards']=copy.deepcopy([w for p in row['distributions'] for w in p['ordered_raw_writes']])
    return document


class ComparisonTests(unittest.TestCase):
    def rejected(self,document):
        with self.assertRaises(ComparisonError):
            comparison(document)

    def test_keccak_ethereum_vectors(self):
        self.assertEqual(keccak(b'').hex(),'c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470')
        self.assertEqual(key(7),'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc')
        self.assertEqual(reward_key('58da990a8f4a3a6ca7cb6315d68a140105917352'),
                         'aa8ff4eb53041f61fcf866c991a1656e1ca7513f7298294a13a54f6be23885a5')

    def test_allowed_map_permutation_only(self):
        for era in ['fixed','minted','dynamic']:
            with self.subTest(era=era):
                original=fixture(era);changed=copy.deepcopy(original)
                start=2 if era=='dynamic' else 1
                writes=changed['periods'][0]['distributions'][0]['ordered_raw_writes']
                writes[start:start+3]=list(reversed(writes[start:start+3]))
                sync(changed)
                raw=lambda d:hashlib.sha256(json.dumps(d,sort_keys=True).encode()).hexdigest()
                self.assertNotEqual(raw(original),raw(changed))
                self.assertEqual(comparison_digest(original),comparison_digest(changed))
                self.assertEqual(original,fixture(era))
                self.assertEqual(comparison(changed)['history_rows'],original['history_rows'])

    def test_author_moved_dropped_or_changed(self):
        for mode in ['move','drop','value']:
            d=fixture();w=d['periods'][0]['distributions'][0]['ordered_raw_writes']
            if mode=='move':w[0],w[1]=w[1],w[0]
            if mode=='drop':w.pop(0)
            if mode=='value':w[0]=pool(A,2,0)
            self.rejected(sync(d))

    def test_zero_map_missing_duplicate_or_value(self):
        for mode in ['missing','duplicate','value']:
            d=fixture();w=d['periods'][0]['distributions'][0]['ordered_raw_writes']
            if mode=='missing':w.pop(3)
            if mode=='duplicate':w[3]=copy.deepcopy(w[2])
            if mode=='value':w[3]=pool(C,1,0)
            self.rejected(sync(d))

    def test_membership_is_not_stats_or_reward_key(self):
        d=fixture();d['inputs']['initial_validators'][-1]['address']='44'*20
        self.rejected(d)
        d=fixture();d['periods'][0]['distributions'][0]['input']['validators'][-1]['validator']='44'*20
        self.rejected(d)
        d=fixture();d['periods'][0]['distributions'][0]['input']['validators'][0]['reward_key']='00'*32
        self.rejected(d)

    def test_suffix_and_yield_barriers(self):
        for era in ['minted','dynamic']:
            for mode in ['drop','move']:
                d=fixture(era);w=d['periods'][0]['distributions'][0]['ordered_raw_writes']
                if mode=='drop':w.pop()
                else:w.insert(0,w.pop())
                self.rejected(sync(d))
        for mode in ['drop','move']:
            d=fixture('dynamic');w=d['periods'][0]['distributions'][0]['ordered_raw_writes']
            if mode=='drop':w.pop(0)
            else:w.append(w.pop(0))
            self.rejected(sync(d))

    def test_flattened_transcript_association(self):
        d=fixture();d['periods'][0]['ordered_raw_writes']['rewards'].pop()
        self.rejected(d)

    def test_reordered_distribution_occurrences(self):
        d=fixture();row=d['periods'][0]
        second=copy.deepcopy(row['distributions'][0]);second['period']=1;second['input']['number']=1
        second['ordered_raw_writes']=[pool(A,12,1),pool(A,22,2),pool(B,14,2),pool(C,0,0)]
        row['distributions'].append(second);row['distribution_order'].append(1);row['reward_output']['minted_reward']='40'
        sync(d)
        comparison(d)
        changed=copy.deepcopy(d);r=changed['periods'][0];r['distributions'].reverse();r['distribution_order'].reverse()
        self.rejected(sync(changed))

    def test_nonreward_physical_and_account_changes_stay_exact(self):
        d=fixture();baseline=comparison_digest(d)
        changed=copy.deepcopy(d);changed['history_rows'][0]['unchanged']='changed'
        self.assertNotEqual(baseline,comparison_digest(changed))
        changed=copy.deepcopy(d);changed['periods'][0]['distributions'][0]['account_operations']=['changed barrier']
        self.assertNotEqual(baseline,comparison_digest(changed))

    def test_provenance_and_begin_stake_changes_fail_closed(self):
        d=fixture();d['periods'][0]['distributions'][0]['ordered_raw_writes'][0]['old']='00'
        self.rejected(sync(d))
        d=fixture();d['periods'][0]['ordered_raw_writes']['begin_block']=[put(validator_key(A),rlp_encode([[6001,1000,0,0],0]))]
        self.rejected(d)

    def test_legacy_validator_record(self):
        d=fixture();old=d['seed']['ordered_raw_writes'][0];old['value']=rlp_encode([6000,1000,0,0]).hex()
        d['seed']['native_catalog']['slots'][0]['value']=old['value']
        comparison(d)

    def test_fork_requires_authenticated_prefix(self):
        self.rejected({'kind':'pre_reward_fork'})



def manual_sha(value):
    # Fixture raw rows/catalogs contain ASCII hex only: exact Go compact sorted
    # JSON SHA256 with no newline. These are declared test facts, not Go results.
    return hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':')).encode()).hexdigest()


def manual_catalog(transcript):
    values={}
    for row in transcript:
        values[(row['address'],row['key'])]=row['value']
    return {'slots':[{'address':a,'key':k,'value':v,'present':bool(v)}
                     for (a,k),v in sorted(values.items())]}


def manual_physical(catalog,period):
    history=[{} for _ in range(5)];latest=[{} for _ in range(5)]
    for slot in catalog['slots']:
        slot_path=keccak(bytes.fromhex(slot['key']))
        db_key=keccak(bytes.fromhex(slot['address'])+slot_path).hex()
        history[4][db_key+period.to_bytes(8,'big').hex()]=slot['value']
        latest[4][db_key]=slot['value']
    return history,latest


def declared_donor(with_distribution=True):
    d=fixture()
    d.update({'scenario':'declared_donor','profile':'declared_unit_fixture','reference':'a'*40,
              'observer':True,'lifecycle':'concrete-flushed'})
    d['seed']['root']='01'*32
    if not with_distribution:
        row=d['periods'][0]
        row['distributions']=[];row['distribution_order']=[]
        row['ordered_raw_writes']['rewards']=[];row['reward_output']['minted_reward']='0'
    catalog=manual_catalog(flatten_donor(d))
    d['periods'][-1]['final']={'root':'02'*32,'committed_root':'02'*32,'native_catalog':catalog}
    d['history_rows'],d['latest_rows']=manual_physical(catalog,d['periods'][-1]['number'])
    return d


def wrap_donor(donor):
    donor=copy.deepcopy(donor)
    last=donor['periods'][-1]
    at=last['number']+6
    transcript=flatten_donor(donor)
    catalog=copy.deepcopy(last['final']['native_catalog'])
    root=last['final']['root']
    identity={'period':last['number'],'root':root,
              'ordered_raw_writes_sha256':manual_sha(transcript),
              'physical_sha256':manual_sha(donor['history_rows']),
              'latest_sha256':manual_sha(donor['latest_rows']),
              'catalog_sha256':manual_sha(catalog)}
    target={key:copy.deepcopy(donor[key]) for key in
            ['schema','configuration','inputs','profile','reference','observer','lifecycle']}
    target['scenario']='target_'+donor['scenario']
    target['seed']={'period':at,'root':root,'ordered_raw_writes':transcript,
                    'history':[{'period':p,'root':root} for p in range(at-5,at+1)],
                    'native_catalog':copy.deepcopy(catalog),'declared_donor_catalog':catalog,
                    'donor_identity':identity,'donor_document':donor}
    target['periods']=[{'number':at+1,'transactions':[],'distributions':[],'distribution_order':[],
                        'ordered_raw_writes':{'begin_block':[],'rewards':[],'end_block':[]},
                        'reward_output':{'minted_reward':'0'},
                        'final':{'root':root,'committed_root':root,'native_catalog':copy.deepcopy(catalog)}}]
    target['history_rows'],target['latest_rows']=manual_physical(catalog,at-5)
    return target


def rebind_donor(target,update_catalog=False):
    donor=target['seed']['donor_document']
    transcript=flatten_donor(donor)
    if update_catalog:
        catalog=manual_catalog(transcript)
        donor['periods'][-1]['final']['native_catalog']=catalog
        donor['history_rows'],donor['latest_rows']=manual_physical(catalog,donor['periods'][-1]['number'])
        target['seed']['native_catalog']=copy.deepcopy(catalog)
        target['seed']['declared_donor_catalog']=copy.deepcopy(catalog)
    target['seed']['ordered_raw_writes']=copy.deepcopy(transcript)
    identity=target['seed']['donor_identity']
    identity['ordered_raw_writes_sha256']=manual_sha(transcript)
    identity['physical_sha256']=manual_sha(donor['history_rows'])
    identity['latest_sha256']=manual_sha(donor['latest_rows'])
    identity['catalog_sha256']=manual_sha(target['seed']['declared_donor_catalog'])
    return target


class DonorComparisonTests(unittest.TestCase):
    def reject(self,document):
        with self.assertRaises(ComparisonError):
            comparison(document)

    def test_recursive_permutation_and_raw_integrity_stay_distinct(self):
        original=wrap_donor(declared_donor())
        changed=copy.deepcopy(original)
        donor=changed['seed']['donor_document']
        rows=donor['periods'][0]['distributions'][0]['ordered_raw_writes']
        rows[1:4]=list(reversed(rows[1:4]));sync(donor);rebind_donor(changed)
        self.assertNotEqual(original['seed']['donor_identity']['ordered_raw_writes_sha256'],
                            changed['seed']['donor_identity']['ordered_raw_writes_sha256'])
        before=copy.deepcopy(changed)
        self.assertEqual(comparison_digest(original),comparison_digest(changed))
        self.assertEqual(changed,before)
        view=comparison(changed)
        self.assertNotIn('ordered_raw_writes_sha256',view['seed']['donor_identity'])
        self.assertIn('ordered_raw_writes_comparison_sha256',view['seed']['donor_identity'])
        for key in ['physical_sha256','latest_sha256','catalog_sha256']:
            self.assertEqual(view['seed']['donor_identity'][key],changed['seed']['donor_identity'][key])

    def test_no_distribution_donor_gains_no_permutation_freedom(self):
        d=wrap_donor(declared_donor(False));view=comparison(d)
        self.assertEqual(view['seed']['ordered_raw_writes'],d['seed']['ordered_raw_writes'])
        changed=copy.deepcopy(d);changed['seed']['ordered_raw_writes'].reverse()
        self.reject(changed)

    def test_raw_digest_and_target_flatten_are_authenticated(self):
        d=wrap_donor(declared_donor())
        changed=copy.deepcopy(d);changed['seed']['donor_identity']['ordered_raw_writes_sha256']='00'*32;self.reject(changed)
        for kind in ['missing','extra','moved']:
            changed=copy.deepcopy(d);rows=changed['seed']['ordered_raw_writes']
            if kind=='missing':rows.pop()
            if kind=='extra':rows.append(copy.deepcopy(rows[-1]))
            if kind=='moved':rows[0],rows[1]=rows[1],rows[0]
            changed['seed']['donor_identity']['ordered_raw_writes_sha256']=manual_sha(rows)
            self.reject(changed)
        changed=copy.deepcopy(d);identity=changed['seed']['donor_identity']
        identity['ordered_raw_writes_comparison_sha256']=identity.pop('ordered_raw_writes_sha256')
        self.reject(changed)

    def test_identity_configuration_and_lifecycle(self):
        d=wrap_donor(declared_donor())
        for key,value in [('period',3),('root','03'*32)]:
            changed=copy.deepcopy(d);changed['seed']['donor_identity'][key]=value;self.reject(changed)
        for key,value in [('reference','b'*40),('observer',False),('lifecycle','batched')]:
            changed=copy.deepcopy(d);changed['seed']['donor_document'][key]=value;self.reject(changed)
        changed=copy.deepcopy(d);changed['seed']['donor_document']['configuration']['dpos']['yield_percentage']=21;self.reject(changed)
        changed=copy.deepcopy(d);changed['seed']['donor_document']['inputs']['genesis_allocations'][0]['balance']='1001';self.reject(changed)
        changed=copy.deepcopy(d);changed['seed']['history'][0]['root']='03'*32;self.reject(changed)
        changed=copy.deepcopy(d);changed['seed']['history'][0]['period']=2;self.reject(changed)

    def test_physical_latest_catalog_hashes_are_never_normalized(self):
        d=wrap_donor(declared_donor())
        for key in ['physical_sha256','latest_sha256','catalog_sha256']:
            changed=copy.deepcopy(d);changed['seed']['donor_identity'][key]='00'*32;self.reject(changed)
        changed=copy.deepcopy(d)
        physical_key=next(iter(changed['seed']['donor_document']['history_rows'][4]))
        changed['seed']['donor_document']['history_rows'][4][physical_key]='ff'
        self.reject(changed)
        changed=copy.deepcopy(d);changed['history_rows'][0]['00'*32]='ff'
        self.assertNotEqual(comparison_digest(changed),comparison_digest(d))

    def test_rehashed_bad_pool_author_or_membership_still_fails_math(self):
        d=wrap_donor(declared_donor())
        for mode in ['value','author','membership']:
            changed=copy.deepcopy(d);donor=changed['seed']['donor_document']
            part=donor['periods'][0]['distributions'][0]
            if mode=='value':part['ordered_raw_writes'][1]=pool(A,99,1)
            if mode=='author':part['ordered_raw_writes'][0],part['ordered_raw_writes'][1]=part['ordered_raw_writes'][1],part['ordered_raw_writes'][0]
            if mode=='membership':part['input']['validators'][0]['validator']='44'*20
            sync(donor);rebind_donor(changed,True)
            self.reject(changed)

    def test_period_phase_and_invocation_association_negatives(self):
        d=wrap_donor(declared_donor())
        for mode in ['duplicate_period','gap','phase','flatten']:
            changed=copy.deepcopy(d);donor=changed['seed']['donor_document']
            if mode=='duplicate_period':donor['periods'].append(copy.deepcopy(donor['periods'][0]))
            if mode=='gap':donor['periods'][0]['number']+=1
            if mode=='phase':donor['periods'][0]['ordered_raw_writes']['extra']=[]
            if mode=='flatten':donor['periods'][0]['ordered_raw_writes']['rewards'].pop()
            self.reject(changed)

    def test_missing_descendant_depth_and_cycle(self):
        d=wrap_donor(declared_donor())
        changed=copy.deepcopy(d);del changed['seed']['donor_document'];self.reject(changed)
        changed=copy.deepcopy(d);del changed['seed']['donor_identity'];self.reject(changed)
        three=wrap_donor(wrap_donor(d))
        comparison(three) # root0 and three donor edges allowed
        self.reject(wrap_donor(three))
        repeated=copy.deepcopy(three)
        repeated['seed']['donor_document']['seed']['donor_identity']=copy.deepcopy(repeated['seed']['donor_identity'])
        self.reject(repeated)
        cycle=copy.deepcopy(d);cycle['seed']['donor_document']=cycle;self.reject(cycle)

    def test_code_objects_are_decoded_before_donor_validation(self):
        from mainnet_era_code_objects import code_objects_binding
        d=wrap_donor(declared_donor(False));raw=copy.deepcopy(d)
        code=b'\x60\x00';hash_value=keccak(code).hex()
        observation={'code':code.hex(),'code_hash':hash_value,'code_size':len(code)}
        raw['periods'][0]['final']['accounts']=[observation]
        encoded=copy.deepcopy(raw)
        row=encoded['periods'][0]['final']['accounts'][0]
        row['code_object']=row['code_hash'];del row['code']
        encoded['code_objects']={hash_value:code.hex()};encoded.update(code_objects_binding(encoded['code_objects']))
        self.assertEqual(comparison_digest(encoded),comparison_digest(raw))
        self.assertIn('code_object',encoded['periods'][0]['final']['accounts'][0])


if __name__=='__main__':
    unittest.main()
