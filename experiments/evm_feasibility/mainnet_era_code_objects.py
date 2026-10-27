"""Strict lossless decoder for schema1 immutable account code objects.

Only the documented account-observation paths are interpreted. Physical history,
rows/latest_rows, logs, calldata and unrelated dictionaries remain unchanged.
A complete assembled body must carry its own closed invocation-local pool;
partial shards must defer resolution until their authenticated base is assembled.
"""
import copy
import hashlib
import json
import re

from mainnet_era_compare import keccak

SCHEMA = 1
MAX_CODE_BYTES = 64 * 1024 * 1024
MAX_OBJECTS = 4096
MAX_ACCOUNT_OCCURRENCES = 4_000_000
MAX_DONOR_DEPTH = 16
METADATA = ('code_objects_schema', 'code_objects', 'code_objects_count', 'code_objects_sha256')


class CodeObjectError(ValueError):
    """Malformed/incomplete packaging; no storage or observation is fabricated."""


def require(condition, message):
    if not condition:
        raise CodeObjectError(message)


def strict_json_loads(raw):
    """Reject duplicate JSON keys BEFORE a parsed pool can hide their conflict."""
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, 'duplicate JSON object key: ' + key)
            result[key] = value
        return result
    def invalid_constant(value):
        raise CodeObjectError('non-JSON numeric constant: ' + value)
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=invalid_constant)


def code_objects_binding(pool):
    """Root/Rust wire: SHA256 of compact sorted JSON stringmap; schema separate."""
    raw = json.dumps(pool,sort_keys=True,separators=(',',':'),ensure_ascii=True).encode()
    return {'code_objects_schema':SCHEMA, 'code_objects_count':len(pool),
            'code_objects_sha256':hashlib.sha256(raw).hexdigest()}


def _mapping(parent, key, path):
    value = parent.get(key) if parent is not None else None
    if value is None:
        return None
    require(type(value) is dict, path + '.' + key + ': expected object')
    return value


def _rows(parent, key, path):
    value = parent.get(key) if parent is not None else None
    if value is None:
        return []
    require(type(value) is list and all(type(row) is dict for row in value),
            path + '.' + key + ': expected object array')
    return value


def _account_occurrences(document):
    result = []
    ancestors = set()
    def add(container, path):
        if container is not None:
            result.extend(_rows(container,'accounts',path))
            require(len(result) <= MAX_ACCOUNT_OCCURRENCES, 'account occurrence bound')
    def transactions(holder, path):
        for i, tx in enumerate(_rows(holder,'transactions',path)):
            tx_path = f'{path}.transactions[{i}]'
            observer = _mapping(tx,'observer',tx_path)
            add(_mapping(observer,'catalog',tx_path+'.observer'),tx_path+'.observer.catalog')
    def walk(doc, path, depth):
        require(type(doc) is dict and depth <= MAX_DONOR_DEPTH, 'document/depth bound')
        require(id(doc) not in ancestors, 'cyclic donor document')
        if depth:
            require(not any(key in doc for key in METADATA), 'nested donor pool/version metadata is forbidden')
        ancestors.add(id(doc))
        genesis = _mapping(doc,'genesis',path)
        add(_mapping(genesis,'native_catalog',path+'.genesis'),path+'.genesis.native_catalog')
        seed = _mapping(doc,'seed',path)
        for name in ('native_catalog','declared_projection_catalog','declared_donor_catalog'):
            add(_mapping(seed,name,path+'.seed'),path+'.seed.'+name)
        donor = _mapping(seed,'donor_document',path+'.seed')
        if donor is not None:
            walk(donor,path+'.seed.donor_document',depth+1)
        for i, period in enumerate(_rows(doc,'periods',path)):
            period_path = f'{path}.periods[{i}]'
            final = _mapping(period,'final',period_path)
            add(final,period_path+'.final')
            add(_mapping(final,'native_catalog',period_path+'.final'),period_path+'.final.native_catalog')
            transactions(period,period_path)
            transactions(_mapping(period,'reward_input',period_path),period_path+'.reward_input')
            for j, part in enumerate(_rows(period,'distributions',period_path)):
                part_path = f'{period_path}.distributions[{j}]'
                transactions(_mapping(part,'input',part_path),part_path+'.input')
        for i, pending in enumerate(_rows(doc,'pending_reward_facts',path)):
            transactions(pending,f'{path}.pending_reward_facts[{i}]')
        pre = _mapping(doc,'pre_reward',path)
        add(pre,path+'.pre_reward')
        add(_mapping(pre,'native_catalog',path+'.pre_reward'),path+'.pre_reward.native_catalog')
        for i, api in enumerate(_rows(doc,'api_cases',path)):
            for name in ('trace','trace_sequence'):
                trace_path = f'{path}.api_cases[{i}].{name}'
                trace = _mapping(api,name,f'{path}.api_cases[{i}]')
                for j, call in enumerate(_rows(trace,'calls',trace_path)):
                    for phase in ('before','after'):
                        phase_path = f'{trace_path}.calls[{j}].{phase}'
                        add(_mapping(call,phase,f'{trace_path}.calls[{j}]'),phase_path)
        ancestors.remove(id(doc))
    walk(document,'document',0)
    return result


def _hex(text, size=None):
    require(type(text) is str and len(text)%2 == 0 and
            re.fullmatch('[0-9a-f]*',text) is not None, 'canonical lowercase hex required')
    require(size is None or len(text) == 2*size, 'hex width')
    return bytes.fromhex(text)


def decode_code_objects(document, *, expected_binding=None):
    """Return a deep copy of the exact raw observation tree.

    New bodies require schema,pool,count,digest. expected_binding may additionally
    supply the root-owned envelope's same three scalar bindings; it never supplies
    missing object bytes. Legacy bodies are unchanged copies only when eligible
    observation paths contain no rogue references or partial codec metadata.
    """
    require(type(document) is dict, 'document object required')
    present = [key in document for key in METADATA]
    require(not any(present) or all(present), 'incomplete codec metadata')
    output = copy.deepcopy(document)
    accounts = _account_occurrences(output)
    if not any(present):
        require(expected_binding is None, 'encoded envelope cannot resolve a legacy body')
        require(all('code_object' not in account for account in accounts), 'legacy document has rogue code reference')
        return output
    require(type(output['code_objects_schema']) is int and output['code_objects_schema'] == SCHEMA,
            'unsupported code-object schema')
    pool = output['code_objects']
    require(type(pool) is dict and len(pool) <= MAX_OBJECTS, 'code-object pool type/count bound')
    require(type(output['code_objects_count']) is int and output['code_objects_count'] == len(pool),
            'code-object count binding')
    _hex(output['code_objects_sha256'],32)
    total = 0
    objects = {}
    for hash_value, text in pool.items():
        _hex(hash_value,32)
        require(type(text) is str and 0 < len(text) <= MAX_CODE_BYTES*2, 'nonempty bounded code object required')
        raw = _hex(text)
        total += len(raw)
        require(total <= MAX_CODE_BYTES, 'decoded pool byte bound')
        require(keccak(raw).hex() == hash_value, 'code object Keccak mismatch')
        objects[hash_value] = raw
    actual_binding = code_objects_binding(pool)
    require(all(output[key] == value for key,value in actual_binding.items()), 'code-object pool digest binding')
    if expected_binding is not None:
        require(type(expected_binding) is dict and all(key in expected_binding for key in actual_binding),
                'complete envelope code-object binding required')
        for key,value in actual_binding.items():
            require(type(expected_binding[key]) is type(value) and expected_binding[key] == value,
                    'cross-invocation/envelope pool binding mismatch')
    replacements = []
    used = set()
    for account in accounts:
        if 'code_object' not in account:
            if 'code' in account:
                require(type(account['code']) is str and account['code'] == '',
                        'encoded body contains unsupported/raw nonempty code observation')
            continue
        require('code' not in account, 'conflicting code and code_object fields')
        reference = account['code_object']
        _hex(reference,32)
        require(reference in objects, 'unknown/missing code object reference')
        require(type(account.get('code_hash')) is str and account['code_hash'] == reference,
                'observation code_hash/reference mismatch')
        size = account.get('code_size')
        require(type(size) is int and 0 <= size <= MAX_CODE_BYTES and size == len(objects[reference]),
                'observation code_size/reference mismatch')
        used.add(reference)
        replacements.append((account,reference))
    require(used == set(objects), 'unused code objects in complete assembled body')
    # Do not look up mutated dictionaries here: JSON aliases may produce the same
    # map at many independently serialized observation locations.
    for account, reference in replacements:
        account.pop('code_object',None)
        account['code'] = pool[reference]
    for key in METADATA:
        del output[key]
    return output
