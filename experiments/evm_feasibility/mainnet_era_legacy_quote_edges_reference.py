#!/usr/bin/env python3
"""Record six independent signed quote-edge transactions for each pinned Go source.

This additive producer records actual transaction-only checkpoints. It never
replaces the original historical family and makes no public native-observer claim.
Run only under an independently admitted source-bound parent.
"""
import base64
import copy
import hashlib
import json
import os
from pathlib import Path
import sys

import mainnet_era_reference as ref
from mainnet_era_profile import REFERENCES, scenarios


def main():
    if len(sys.argv) != 1:
        raise ValueError('no command-line overrides are accepted')
    launch = os.environ.get('RUSTAXA_ERA_LAUNCH_MANIFEST')
    if not launch:
        raise ValueError('source-bound launch required')
    context = json.loads(Path(launch).read_bytes())
    if ref.cutover_context(context) is None:
        raise ValueError('schema2 source-bound launch required')
    artifact = Path(os.environ['RUN_ARTIFACT_DIR']).resolve(strict=True)
    original = next(s for s in scenarios() if s['name'] == 'claim_all_legacy_batch_correction')
    scenario = copy.deepcopy(original)
    scenario['name'] = 'claim_all_legacy_direct_quote_edges'
    schedule = ref.schedule_for(original, artifact)
    outputs = {}
    for label in ('local', 'public'):
        data, directory = ref.run(REFERENCES[label], label, scenario, schedule, artifact,
                                  selector={'kind': 'quote_edges'})
        if data.get('component') != scenario['name'] or data.get('mode') != label:
            raise ValueError('actual source composition/component mismatch')
        if data.get('reference') != REFERENCES[label] or len(data.get('cases', [])) != 6:
            raise ValueError('actual source pin/case cardinality mismatch')
        identities = {(row['batch'], row['offset']) for row in data['cases']}
        if identities != {(batch, offset) for batch in (0, 1) for offset in (-1, 0, 1)}:
            raise ValueError('actual case identity mismatch')
        for row in data['cases']:
            batch, offset = row['batch'], row['offset']
            quote = 90000 if batch == 0 else 45000
            expected_input = bytes.fromhex('09b72e00') + batch.to_bytes(32, 'big')
            if row['basis_digest'] != data['basis_digest'] or row['requested_native_gas'] != quote + offset:
                raise ValueError('cold branch basis/input mismatch')
            if row['signed_gas'] != row['intrinsic'] + row['requested_native_gas']:
                raise ValueError('signed/intrinsic/native input relation mismatch')
            if label == 'local':
                invocations = row['native_invocations']
                if not isinstance(invocations, list) or len(invocations) != 1:
                    raise ValueError('actual local native observation missing')
                actual = invocations[0]
                if (actual['SuppliedGas'] != row['requested_native_gas'] or
                    actual['RequiredGas'] != quote or actual['Depth'] != 0 or
                    actual['Value'] != 0 or base64.b64decode(actual['Input'], validate=True) != expected_input or
                    actual['Caller'].lower().removeprefix('0x') != row['sender']):
                    raise ValueError('actual local native frame/quote mismatch')
            elif row['native_frame_visibility'] != 'public_unobserved' or row['native_invocations'] is not None:
                raise ValueError('public native observation must be explicitly unobserved')
        body = (json.dumps(data, sort_keys=True, separators=(',', ':')) + '\n').encode()
        target = artifact / (label + '-legacy-quote-edges-' + hashlib.sha256(body).hexdigest() + '.json')
        with target.open('xb') as out:
            out.write(body)
            out.flush()
            os.fsync(out.fileno())
        outputs[label] = {'path': str(target), 'bytes': len(body),
                          'sha256': hashlib.sha256(body).hexdigest(),
                          'producer_directory': str(directory)}
    print(json.dumps({'state': 'ACTUAL_REFERENCE_REQUIRES_INDEPENDENT_REVIEW',
                      'component': scenario['name'], 'outputs': outputs}, sort_keys=True))


if __name__ == '__main__':
    main()
