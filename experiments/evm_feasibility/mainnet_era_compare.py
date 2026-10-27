#!/usr/bin/env python3
"""Validated, same-pin reward-map comparison view. Raw documents stay unchanged.

Authority: shard-contract-review.md and pre-reward-and-legacy-map-review.md in
/home/fry/artifacts/mainnet-era-20261026-01a179de. Only validator reward-map Put
order within an authenticated invocation may change. Physical histories, roots,
account effects, source-period vectors and all other fields remain exact.

Keccak round constants/round semantics adapted from Go Authors' BSD-licensed
 golang.org/x/crypto@v0.23.0/sha3/keccakf.go (Go module pinned by taraxa go.mod):
 SHA256 905f7cd07071da81fdf687fe2f7ffd63953db93578573c49d9d9c26c0352f4f5.
Legacy padding/rate from hashes.go SHA256
 a584e638d721c63812030f211912494cda9cea1d560dadab46ec590b98e83979.
Copyright 2014 The Go Authors. All rights reserved. BSD-style license applies
to that adaptation; the original module LICENSE supplies its terms.
"""
# Copyright (c) 2009 The Go Authors. All rights reserved.
#
# Redistribution and use in source and binary forms, with or without
# modification, are permitted provided that the following conditions are
# met:
#
#    * Redistributions of source code must retain the above copyright
# notice, this list of conditions and the following disclaimer.
#    * Redistributions in binary form must reproduce the above
# copyright notice, this list of conditions and the following disclaimer
# in the documentation and/or other materials provided with the
# distribution.
#    * Neither the name of Google Inc. nor the names of its
# contributors may be used to endorse or promote products derived from
# this software without specific prior written permission.
#
# THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
# "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
# LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
# A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
# OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
# SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
# LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
# DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
# THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
# (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
# OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
import argparse
import copy
from functools import lru_cache
import gzip
import hashlib
import json
from pathlib import Path

VERSION = 'mainnet-era-reward-comparison-v2'
MAX_DONOR_DEPTH = 3
DPOS = '00000000000000000000000000000000000000fe'
MAX256 = 2**256 - 1
MASK64 = 2**64 - 1
RC = (0x1, 0x8082, 0x800000000000808a, 0x8000000080008000,
      0x808b, 0x80000001, 0x8000000080008081, 0x8000000000008009,
      0x8a, 0x88, 0x80008009, 0x8000000a, 0x8000808b,
      0x800000000000008b, 0x8000000000008089, 0x8000000000008003,
      0x8000000000008002, 0x8000000000000080, 0x800a,
      0x800000008000000a, 0x8000000080008081, 0x8000000000008080,
      0x80000001, 0x8000000080008008)
ROT = ((0,36,3,41,18), (1,44,10,45,2), (62,6,43,15,61),
       (28,55,25,21,56), (27,20,39,8,14))


class ComparisonError(ValueError):
    """Unsupported or invalid evidence; never repaired by normalization."""


def require(condition, message):
    if not condition:
        raise ComparisonError(message)


def bounded(value, bits=256):
    require(type(value) is int and 0 <= value < 2**bits, 'arithmetic width/underflow')
    return value


def mul(a, b):
    return bounded(a*b)


def add(a, b):
    return bounded(a+b)


def div(a, b):
    require(b > 0, 'zero denominator')
    return a//b


def rot(x, n):
    return ((x << n) | (x >> ((64-n) % 64))) & MASK64


@lru_cache(maxsize=4096)
def keccak(raw):
    """Ethereum Keccak-256: rate136, legacy suffix0x01 (not SHA3-256)."""
    require(isinstance(raw, bytes), 'Keccak bytes required')
    padded = bytearray(raw)
    padded.append(1)
    padded.extend(bytes((-len(padded)) % 136))
    padded[-1] |= 0x80
    state = [0]*25
    for at in range(0,len(padded),136):
        chunk = padded[at:at+136]
        for i in range(17):
            state[i] ^= int.from_bytes(chunk[8*i:8*i+8], 'little')
        for rc in RC:
            c = [state[x]^state[x+5]^state[x+10]^state[x+15]^state[x+20] for x in range(5)]
            d = [c[(x-1)%5]^rot(c[(x+1)%5],1) for x in range(5)]
            b = [0]*25
            for x in range(5):
                for y in range(5):
                    b[y+5*((2*x+3*y)%5)] = rot(state[x+5*y]^d[x],ROT[x][y])
            for x in range(5):
                for y in range(5):
                    state[x+5*y] = b[x+5*y]^((~b[(x+1)%5+5*y])&b[(x+2)%5+5*y])
            state[0] ^= rc
    return b''.join(v.to_bytes(8,'little') for v in state)[:32]


def hexbytes(value, size=None):
    require(isinstance(value,str) and len(value)%2 == 0 and value == value.lower(), 'canonical hex required')
    try:
        raw = bytes.fromhex(value)
    except ValueError as error:
        raise ComparisonError('invalid hex') from error
    require(raw.hex() == value and (size is None or len(raw) == size), 'hex identity/width')
    return raw


def key(field):
    return keccak(bytes([field])).hex()


def validator_key(address):
    return keccak(b'\x00\x00' + hexbytes(address,20)).hex()


def reward_key(address):
    return keccak(b'\x00\x02' + hexbytes(address,20)).hex()


def compact(value):
    bounded(value)
    return value.to_bytes((value.bit_length()+7)//8,'big')


def rlp_encode(value):
    if type(value) is int:
        value = compact(value)
    if isinstance(value,list):
        raw = b''.join(rlp_encode(v) for v in value)
        short,long = 0xc0,0xf7
    else:
        require(isinstance(value,bytes), 'RLP input')
        raw = value
        if len(raw) == 1 and raw[0] < 0x80:
            return raw
        short,long = 0x80,0xb7
    if len(raw) <= 55:
        return bytes([short+len(raw)]) + raw
    length = compact(len(raw))
    return bytes([long+len(length)]) + length + raw


def rlp_decode(raw):
    def item(at, depth=0):
        require(depth <= 4 and at < len(raw), 'RLP depth/truncation')
        first = raw[at]
        if first < 0x80:
            return raw[at:at+1], at+1
        if first <= 0xb7:
            start,end = at+1,at+1+first-0x80
            require(end <= len(raw), 'RLP string truncation')
            value = raw[start:end]
            require(not(len(value)==1 and value[0]<0x80), 'RLP noncanonical short string')
            return value,end
        if first <= 0xbf:
            width = first-0xb7
            require(at+1+width <= len(raw) and raw[at+1] != 0, 'RLP length')
            length = int.from_bytes(raw[at+1:at+1+width],'big')
            require(length > 55, 'RLP noncanonical long string')
            end = at+1+width+length
            require(end <= len(raw), 'RLP long string truncation')
            return raw[at+1+width:end],end
        if first <= 0xf7:
            start,end = at+1,at+1+first-0xc0
        else:
            width=first-0xf7
            require(at+1+width <= len(raw) and raw[at+1] != 0, 'RLP list length')
            length=int.from_bytes(raw[at+1:at+1+width],'big')
            require(length > 55, 'RLP noncanonical long list')
            start,end=at+1+width,at+1+width+length
        require(end <= len(raw), 'RLP list truncation')
        values=[]
        while start<end:
            value,start=item(start,depth+1)
            values.append(value)
        require(start==end, 'RLP list boundary')
        return values,end
    value,end=item(0)
    require(end==len(raw) and rlp_encode(value)==raw, 'RLP trailing/noncanonical bytes')
    return value


def integer(value,bits=256):
    require(isinstance(value,bytes) and (not value or value[0] != 0), 'noncanonical RLP integer')
    return bounded(int.from_bytes(value,'big'),bits)


def validator(raw):
    """GetValidator's exact legacy four-field or nested extended record."""
    values=rlp_decode(raw)
    require(isinstance(values,list), 'validator record list')
    if len(values)==2 and isinstance(values[0],list):
        integer(values[1],16)
        values=values[0]
    require(len(values)==4, 'unsupported validator record')
    stake=integer(values[0])
    commission=integer(values[1],16)
    require(commission <= 10000, 'commission outside valid comparison domain')
    integer(values[2],64)
    integer(values[3],64)
    return stake,commission


class Ledger:
    def __init__(self,document, donor_addresses=None):
        self.rows={}
        self.addresses=set()
        initial=document['inputs']['initial_validators']
        require(initial and len({v['address'] for v in initial})==len(initial), 'complete distinct initial validators required')
        for value in initial:
            self.add_address(value['address'])
        if donor_addresses is not None:
            for address in donor_addresses:
                self.add_address(address)
        self.apply(document['seed']['ordered_raw_writes'])
        # Seed writes are replayed, then checked against the declared complete
        # native inventory; neither a reward-pool key nor a stats map proves existence.
        catalog=document['seed']['native_catalog']['slots']
        known=set()
        for slot in catalog:
            identity=(slot['address'],slot['key'])
            hexbytes(identity[0],20);hexbytes(identity[1],32)
            require(identity not in known, 'duplicate seed inventory')
            known.add(identity)
            raw=hexbytes(slot.get('value',''))
            if slot['present']:
                require(bool(raw) and self.rows.get(identity)==raw, 'seed raw-write/inventory mismatch')
            else:
                require(not self.rows.get(identity), 'forged absent seed slot')
        require(all(identity in known for identity in self.rows), 'seed inventory incomplete')
        for address in self.addresses:
            if donor_addresses is None:
                require(self.get(validator_key(address)), 'initial validator record missing')
            self.validator(address)

    def add_address(self,address):
        hexbytes(address,20)
        self.addresses.add(address)

    def get(self,k):
        return self.rows.get((DPOS,k),b'')

    def validator(self,address):
        require(address in self.addresses, 'validator outside authenticated address domain')
        raw=self.get(validator_key(address))
        return validator(raw) if raw else None

    def stake(self):
        return sum((self.validator(a) or (0,0))[0] for a in self.addresses)

    def apply(self,writes):
        require(isinstance(writes,list), 'raw transcript array')
        for row in writes:
            # Additional old/read/operation shapes need explicit validation.
            # Fail closed rather than ignore their provenance.
            require(set(row)=={'address','key','value'}, 'unsupported raw Put/provenance shape')
            hexbytes(row['address'],20);hexbytes(row['key'],32)
            raw=hexbytes(row['value'])
            identity=(row['address'],row['key'])
            if raw:
                self.rows[identity]=raw
            else:
                self.rows.pop(identity,None)

    def pool(self,address):
        raw=self.get(reward_key(address))
        require(bool(raw), 'existing validator reward pool missing')
        value=rlp_decode(raw)
        require(isinstance(value,list) and len(value)==2, 'reward pool shape')
        return [integer(v) for v in value]

    def check_put(self,row,k,value):
        require(set(row)=={'address','key','value'}, 'unsupported reward Put/provenance shape')
        require(row['address']==DPOS and row['key']==k and row['value']==value.hex(), 'reward mutation key/value/barrier mismatch')
        self.apply([row])

    def add_pool(self,row,address,commission_reward,delegator_reward):
        pool=self.pool(address)
        expected=rlp_encode([add(pool[0],delegator_reward),add(pool[1],commission_reward)])
        self.check_put(row,reward_key(address),expected)


def distribution(ledger,configuration,execution_period,part,delegated):
    """Validate every occurrence before sorting only the distinct map segment."""
    stats=part['input']
    require(stats['number']==part['period'], 'distribution source identity')
    writes=part['ordered_raw_writes']
    require(isinstance(writes,list), 'distribution writes')
    hf=configuration['hardforks'];dpos=configuration['dpos']
    aspen1=int(hf['aspen']['part_one_block']);aspen2=int(hf['aspen']['part_two_block'])
    offset=0
    def consume(k,value):
        nonlocal offset
        require(offset<len(writes), 'missing reward prefix/suffix/author row')
        ledger.check_put(writes[offset],k,value)
        offset+=1
    supply=0
    if execution_period >= aspen2:
        raw_supply=ledger.get(key(7))
        if not raw_supply:
            minted=int.from_bytes(ledger.get(key(6)),'big')
            allocations=configuration.get('genesis_balances_sum')
            # The normal exported configuration omits this redundant sum; the
            # caller supplies the authenticated declared allocation sum below.
            require(allocations is not None, 'missing authenticated genesis allocation sum')
            supply=add(add(int(allocations),minted),int(hf['aspen']['generated_rewards']))
            consume(key(7),compact(supply))
            consume(key(6),b'')
        else:
            supply=bounded(int.from_bytes(raw_supply,'big'))
        maximum=int(hf['aspen']['max_supply'])
        require(0<supply<=maximum, 'invalid dynamic supply')
        current_yield=div(mul(maximum-supply,1000000),supply)
        bounded(current_yield,64)
        consume(key(8),rlp_encode(current_yield))
        rate=int(stats['blocks_per_year']) if execution_period>=int(hf['cacti']['block']) else int(dpos['blocks_per_year'])
        bounded(rate,32)
        block_reward=div(mul(delegated,current_yield),mul(1000000,rate))
    else:
        block_reward=div(mul(delegated,int(dpos['yield_percentage'])),mul(100,int(dpos['blocks_per_year'])))
    total_votes=bounded(int(stats['total_votes_weight']),64)
    max_votes=bounded(int(stats['max_votes_weight']),64)
    total_dag=bounded(int(stats['total_dag_blocks_count']),32)
    dag_reward=block_reward
    votes_reward=0;bonus=0
    if total_votes:
        dag_reward=div(mul(block_reward,int(dpos['dag_proposers_reward_percent'])),100)
        base=div(mul(block_reward,int(dpos['max_block_author_reward_percent'])),100)
        votes_reward=bounded(block_reward-dag_reward-base)
        maximum=max(max_votes,total_votes)
        if maximum==total_votes:
            bonus=base
        else:
            threshold=bounded(maximum*2,64)//3+1
            bonus=div(mul(base,max(total_votes-threshold,0)),maximum-threshold)
    author=stats['block_author']
    author_record=ledger.validator(author)
    minted=0
    if bonus and author_record is not None:
        commission=div(mul(bonus,author_record[1]),10000)
        require(offset<len(writes), 'missing author row')
        ledger.add_pool(writes[offset],author,commission,bonus-commission)
        offset+=1;minted=bonus
    map_start=offset
    validators=stats['validators']
    require(isinstance(validators,list), 'validator stats vector')
    expected={}
    seen=set();dag_sum=0;vote_sum=0
    for row in validators:
        address=row['validator']
        require(address not in seen, 'duplicate input validator map key')
        seen.add(address)
        record=ledger.validator(address)
        if 'reward_key' in row:
            require(row['reward_key']==reward_key(address), 'forged reward key metadata')
        dag=bounded(int(row['dag_blocks_count']),32)
        vote=bounded(int(row['vote_weight']),64)
        dag_sum+=dag;vote_sum+=vote
        reward=0
        if dag:
            reward=div(mul(dag,dag_reward),total_dag)
        if vote:
            reward=add(reward,div(mul(vote,votes_reward),total_votes))
        if record is None:
            continue
        commission=div(mul(reward,record[1]),10000)
        fee=bounded(int(row.get('fees_reward') or 0))
        expected[reward_key(address)]=(address,add(commission,fee),reward-commission)
        minted=add(minted,reward)
    require(dag_sum==total_dag and vote_sum==total_votes, 'invalid stats totals')
    require(minted<=block_reward, 'invalid reward arithmetic')
    require(offset+len(expected)<=len(writes), 'missing map write, including zero write')
    observed=set()
    for row in writes[offset:offset+len(expected)]:
        require(row['key'] in expected and row['key'] not in observed, 'unknown/duplicate/moved reward map row')
        observed.add(row['key'])
        address,commission,reward=expected[row['key']]
        ledger.add_pool(row,address,commission,reward)
    offset+=len(expected)
    require(observed==set(expected), 'incomplete existing-validator map')
    map_end=offset
    if execution_period>=aspen2:
        consume(key(7),compact(add(supply,minted)))
    elif execution_period>=aspen1:
        old=int.from_bytes(ledger.get(key(6)),'big')
        consume(key(6),compact(add(old,minted)))
    require(offset==len(writes), 'extra/moved suffix or cross-invocation mutation')
    require(int(part['minted_reward'])==minted, 'minted amount mismatch')
    # All raw row dictionaries are retained. Only the validated map array order
    # changes. Prefix/author/suffix occurrences stay in their original positions.
    return writes[:map_start]+sorted(writes[map_start:map_end],key=lambda row:row['key'])+writes[map_end:]


def donor_json_digest(value):
    """Go json.Marshal SHA256: sorted compact JSON, no newline; raw writes ASCII.

    HTML/U+2028/U+2029 escapes also match Go for catalog identity material.
    The comparison transcript uses this same format; overall VERSIONv2 binds
    the comparison rules separately from retained raw-integrity identities.
    """
    raw=json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False,allow_nan=False)
    for literal,escaped in [('&',r'\u0026'),('<',r'\u003c'),('>',r'\u003e'),
                            ('\u2028',r'\u2028'),('\u2029',r'\u2029')]:
        raw=raw.replace(literal,escaped)
    return hashlib.sha256(raw.encode()).hexdigest()


def _write_rows(value):
    require(type(value) is list, 'donor transcript array required')
    for row in value:
        require(type(row) is dict and set(row)=={'address','key','value'}, 'unsupported donor raw Put/provenance shape')
        hexbytes(row['address'],20);hexbytes(row['key'],32);hexbytes(row['value'])
    return value


def flatten_donor(document):
    """Seed, then chronological begin/transactions/invocations/end, once each."""
    require(type(document) is dict and type(document.get('seed')) is dict, 'complete donor seed required')
    seed=document['seed']
    previous=bounded(seed['period'],64)
    result=list(_write_rows(seed['ordered_raw_writes']))
    periods=document.get('periods')
    require(type(periods) is list and periods, 'complete nonempty donor periods required')
    for period in periods:
        require(type(period) is dict and bounded(period['number'],64)==previous+1, 'donor period gap/duplicate/order')
        previous=period['number']
        phases=period.get('ordered_raw_writes')
        require(type(phases) is dict and set(phases)=={'begin_block','rewards','end_block'}, 'donor exact phase schema')
        result.extend(_write_rows(phases['begin_block']))
        txs=period.get('transactions')
        require(type(txs) is list, 'donor transaction vector required')
        for index,tx in enumerate(txs):
            require(type(tx) is dict, 'donor transaction shape')
            if 'index' in tx:
                require(type(tx['index']) is int and tx['index']==index, 'donor transaction index gap/duplicate')
            result.extend(_write_rows(tx['ordered_raw_writes']))
        parts=period.get('distributions');order=period.get('distribution_order')
        require(type(parts) is list and type(order) is list and order==[part['period'] for part in parts],
                'donor source-period vector mismatch')
        require(len(order)==len(set(order)), 'duplicate donor source periods')
        rewards=[]
        for part in parts:
            require(type(part) is dict and part['input']['number']==part['period'], 'donor invocation/source-period identity')
            rewards.extend(_write_rows(part['ordered_raw_writes']))
        require(rewards==_write_rows(phases['rewards']), 'donor redundant reward flatten mismatch')
        result.extend(rewards)
        result.extend(_write_rows(phases['end_block']))
    return result


def _donor_tree_guard(document,depth=0,ancestors=None,identities=None):
    """Reject cycles/depth before any deepcopy or code-object traversal."""
    require(type(document) is dict and depth<=MAX_DONOR_DEPTH, 'maximum three donor edges exceeded')
    ancestors=set() if ancestors is None else ancestors
    identities=set() if identities is None else identities
    require(id(document) not in ancestors, 'cyclic donor document')
    ancestors.add(id(document))
    seed=document.get('seed')
    if type(seed) is dict:
        child=seed.get('donor_document');claim=seed.get('donor_identity')
        require((child is None)==(claim is None), 'missing donor document or identity')
        if child is not None:
            require(type(child) is dict and type(claim) is dict, 'unknown donor reference representation')
            identity=(claim.get('period'),claim.get('root'))
            require(type(identity[0]) is int and type(identity[1]) is str, 'donor identity scalar types')
            require(identity not in identities, 'repeated unresolved donor identity')
            identities.add(identity)
            _donor_tree_guard(child,depth+1,ancestors,identities)
            identities.remove(identity)
    ancestors.remove(id(document))


def _latest_from_physical(history,final_period):
    """Validate/reconstruct only an identity digest; never rewrite history rows."""
    require(type(history) is list and len(history)==5 and all(type(c) is dict for c in history),
            'complete five-column donor physical history required')
    latest=[]
    for column,rows in enumerate(history):
        selected={};versions={}
        for encoded,value in rows.items():
            raw=hexbytes(encoded,40 if column in (2,4) else 32)
            hexbytes(value)
            if column in (2,4):
                period=int.from_bytes(raw[32:],'big')
                require(period<=final_period, 'donor future/uncommitted physical history')
                k=raw[:32].hex()
                if k not in versions or period>versions[k]:
                    versions[k]=period;selected[k]=value
            else:
                selected[encoded]=value
        latest.append(selected)
    return latest


def _qualified_donor(document,depth):
    seed=document['seed'];donor=seed.get('donor_document')
    if donor is None:
        return None
    identity=seed['donor_identity']
    require(type(document.get('schema')) is int and document['schema']==1
            and type(donor.get('schema')) is int and donor['schema']==1, 'unknown donor document schema')
    require(donor.get('kind')!='pre_reward_fork', 'donor must be fully committed')
    require(donor.get('configuration')==document['configuration'] and donor.get('inputs')==document['inputs'],
            'donor config/input substitution')
    # Scenario names intentionally differ. All available pin/lifecycle/profile
    # labels must agree in presence and value; absent labels cannot be invented.
    for label in ('reference','reference_pin','profile','observer','lifecycle','execution_lifecycle','cleanup_mode'):
        require((label in donor)==(label in document) and type(donor.get(label)) is type(document.get(label))
                and donor.get(label)==document.get(label),
                'donor reference/lifecycle semantic label mismatch: '+label)
    if 'scenario' in document or 'scenario' in donor:
        require(type(document.get('scenario')) is str and bool(document['scenario'])
                and type(donor.get('scenario')) is str and bool(donor['scenario']), 'donor scenario identity')
    raw=flatten_donor(donor)
    require(seed['ordered_raw_writes']==raw, 'target seed differs from complete raw donor flatten')
    require('ordered_raw_writes_comparison_sha256' not in identity, 'comparison digest cannot replace raw donor integrity')
    hexbytes(identity['ordered_raw_writes_sha256'],32)
    require(identity['ordered_raw_writes_sha256']==donor_json_digest(raw), 'raw donor transcript digest mismatch')
    last=donor['periods'][-1];final=last.get('final')
    require(type(final) is dict and type(identity.get('period')) is int
            and identity['period']==last['number'], 'donor final period identity mismatch')
    root=identity['root'];hexbytes(root,32)
    require(final.get('root')==root and final.get('committed_root')==root, 'donor final committed root mismatch')
    history=seed.get('history')
    require(type(history) is list and len(history)==6, 'six explicit donor target retained identities required')
    first=bounded(history[0]['period'],64)
    require(identity['period']<first and history[-1]['period']==seed['period'], 'donor/target time chronology')
    require(seed.get('root')==root and all(type(row) is dict and type(row['period']) is int
                and row['period']==first+i and row.get('root')==root for i,row in enumerate(history)),
            'computed source/target seed root identities mismatch')
    physical=donor.get('history_rows')
    latest=_latest_from_physical(physical,last['number'])
    for field in ('physical_sha256','latest_sha256','catalog_sha256'):
        hexbytes(identity[field],32)
    require(identity['physical_sha256']==donor_json_digest(physical), 'donor physical identity mismatch')
    require(identity['latest_sha256']==donor_json_digest(latest), 'donor latest identity mismatch')
    if 'latest_rows' in donor:
        require(donor['latest_rows']==latest, 'donor latest rows disagree with physical history')
    catalog=seed.get('declared_donor_catalog')
    require(type(catalog) is dict and catalog==seed['native_catalog'] and catalog==final.get('native_catalog'),
            'donor/target raw catalog identity mismatch')
    require(identity['catalog_sha256']==donor_json_digest(catalog), 'donor catalog identity mismatch')
    view,addresses=_comparison(donor,depth+1)
    normalized=flatten_donor(view)
    return view,addresses,normalized


def _comparison(document,depth=0):
    """Deep-copy and normalize a complete normal document under the narrow rule.

    Fork documents deliberately raise: the lead must authenticate their normal
    prefix and replace only the validated prefix comparison view. No seed or
    absent fields are manufactured for a fork.
    """
    require(document.get('kind')!='pre_reward_fork', 'fork needs authenticated normal-prefix handling by caller')
    require('seed' in document and 'periods' in document and 'inputs' in document, 'complete normal document required')
    require(keccak(b'').hex()=='c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470', 'Keccak empty-vector guard')
    require(key(7)=='ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc', 'Keccak supply-vector guard')
    qualified=_qualified_donor(document,depth)
    output=copy.deepcopy(document)
    donor_addresses=None
    if qualified is not None:
        donor_view,donor_addresses,normalized_seed=qualified
        output['seed']['donor_document']=donor_view
        output['seed']['ordered_raw_writes']=copy.deepcopy(normalized_seed)
        identity=output['seed']['donor_identity']
        del identity['ordered_raw_writes_sha256']
        identity['ordered_raw_writes_comparison_sha256']=donor_json_digest(normalized_seed)
    # Always replay the original RAW target seed against its original catalog.
    # Normalized donor bytes are a comparison view, never state authority.
    ledger=Ledger(document,donor_addresses)
    configuration=copy.deepcopy(document['configuration'])
    total=sum(int(row['balance']) for row in document['inputs']['genesis_allocations'])
    if 'genesis_balances_sum' in configuration:
        require(int(configuration['genesis_balances_sum'])==total, 'genesis allocation sum mismatch')
    configuration['genesis_balances_sum']=bounded(total)
    register=keccak(b'registerValidator(address,bytes,bytes,uint16,string,string)')[:4]
    previous=int(document['seed']['period'])
    require(document['periods'], 'empty period evidence')
    for source,target in zip(document['periods'],output['periods'],strict=True):
        period=source['number']
        require(type(period) is int and period==previous+1, 'nonchronological period evidence')
        previous=period
        phases=source['ordered_raw_writes']
        require(set(phases)=={'begin_block','rewards','end_block'}, 'unsupported period phases')
        committed_amount=bounded(int.from_bytes(ledger.get(key(5)),'big'))
        starting_stake=ledger.stake()
        ledger.apply(phases['begin_block'])
        # The pinned Go correction runs in EndBlock after counter publication.
        # No source-authorized BeginBlock stake mutation exists in this contract.
        require(ledger.stake()==starting_stake, 'unsupported BeginBlock stake/system change')
        for tx in source['transactions']:
            calldata=hexbytes(tx['input'])
            if tx.get('to')==DPOS and calldata[:4]==register:
                require(len(calldata)>=36 and not any(calldata[4:16]), 'registerValidator ABI address')
                ledger.add_address(calldata[16:36].hex())
            ledger.apply(tx['ordered_raw_writes'])
        # source field[5] is deferred until EndBlock; live TotalStake writes
        # reflect registrations, delegate/custody/redelegate changes immediately.
        delegated=bounded(committed_amount+ledger.stake()-starting_stake)
        require(ledger.get(key(5))==compact(committed_amount), 'unsupported early delegated-counter publication')
        parts=source['distributions']
        require(source['distribution_order']==[part['period'] for part in parts], 'distribution source-vector mismatch')
        flattened=[row for part in parts for row in part['ordered_raw_writes']]
        require(flattened==phases['rewards'], 'flattened/distribution transcript mismatch')
        new=[];minted=0
        for original,changed in zip(parts,target['distributions'],strict=True):
            normalized=distribution(ledger,configuration,period,original,delegated)
            changed['ordered_raw_writes']=copy.deepcopy(normalized)
            new.extend(copy.deepcopy(normalized))
            minted=add(minted,int(original['minted_reward']))
        target['ordered_raw_writes']['rewards']=new
        if 'reward_output' in source:
            require(int(source['reward_output']['minted_reward'])==minted, 'period minted total mismatch')
        before_end={a:ledger.validator(a) for a in ledger.addresses}
        ledger.apply(phases['end_block'])
        changed={a:((before_end[a] or (0,0))[0]-(ledger.validator(a) or (0,0))[0])
                 for a in ledger.addresses
                 if (before_end[a] or (0,0))[0] != (ledger.validator(a) or (0,0))[0]}
        if changed:
            hf=configuration['hardforks']
            require(period==int(hf.get('fix_redelegate_block',-1)), 'unsupported EndBlock stake/system mutation')
            corrections={}
            for item in hf['redelegations']:
                address=item['validator']
                require(address in ledger.addresses, 'correction validator outside seed domain')
                corrections[address]=corrections.get(address,0)+int(item['amount'])
            require(changed=={a:n for a,n in corrections.items() if n}, 'correction stake delta differs from pinned policy')
        require(int.from_bytes(ledger.get(key(5)),'big')==delegated, 'EndBlock delegated-counter mismatch')
    return output,set(ledger.addresses)


def comparison(document):
    """Pure v2 view: lossless code decode, then validated bounded donor recursion."""
    _donor_tree_guard(document)
    # Local import is required: the codec itself imports this module's Keccak.
    from mainnet_era_code_objects import decode_code_objects
    try:
        decoded=decode_code_objects(document)
    except ValueError as error:
        raise ComparisonError(str(error)) from error
    try:
        return _comparison(decoded)[0]
    except (KeyError,TypeError,IndexError) as error:
        raise ComparisonError('malformed donor/documented comparison schema: '+str(error)) from error


def comparison_digest(document):
    view=comparison(document)
    raw=json.dumps(view,sort_keys=True,separators=(',',':'),ensure_ascii=True).encode()
    return hashlib.sha256(VERSION.encode()+b'\x00'+raw).hexdigest()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('document',type=Path)
    args=parser.parse_args()
    raw=args.document.read_bytes()
    if args.document.suffix=='.gz':
        raw=gzip.decompress(raw)
    print(comparison_digest(json.loads(raw)))


if __name__=='__main__':
    main()
