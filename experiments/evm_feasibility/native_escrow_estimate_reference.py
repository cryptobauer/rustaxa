#!/usr/bin/env python3
"""Compare escrow probes from actual pinned DryRunner with unchanged C++ search."""
import argparse
import hashlib
import json
from pathlib import Path

from reference import REVISIONS
import native_metadata_estimate_reference as support

HERE = Path(__file__).resolve().parent
INPUT = HERE / 'fixtures/native_escrow_simulation/public.json'
FIXTURES = HERE / 'fixtures/native_escrow_estimate'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    requests = json.loads(INPUT.read_bytes())['cases']
    outputs = {label: support.run_reference(revision, requests) for label, revision in REVISIONS.items()}
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Pinned escrow estimate probes differ')
    documents = [json.loads(data) for data in outputs.values()]
    for document in documents:
        if (document['schema'] != 1 or document['state_before'] != document['state_after']
                or [case['name'] for case in document['cases']] != [case['name'] for case in requests]):
            raise RuntimeError('Invalid escrow estimate corpus')
    # This compiles the unchanged upstream search body. Its callback checks every
    # requested gas against the Go transcript and requires all probes consumed.
    cpp = support.cpp_reference(documents[0])
    outputs['cpp'] = (json.dumps(cpp, sort_keys=True, indent=2) + '\n').encode()
    manifest = {'schema': 1, 'references': REVISIONS,
                'source_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in support.SOURCES},
                'support_harness_sha256': hashlib.sha256(Path(support.__file__).read_bytes()).hexdigest(),
                'input_sha256': hashlib.sha256(INPUT.read_bytes()).hexdigest(),
                'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                'algorithm_sha256': cpp['algorithm_sha256'],
                'sha256': {label: hashlib.sha256(data).hexdigest() for label, data in outputs.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for label, data in outputs.items():
            (FIXTURES / f'{label}.json').write_bytes(data)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Escrow estimate manifest changed')
        for label, data in outputs.items():
            if data != (FIXTURES / f'{label}.json').read_bytes():
                raise RuntimeError(f'{label}: escrow estimate changed')
    print('Both pinned escrow probe corpora match the unchanged C++ search')


if __name__ == '__main__':
    main()
