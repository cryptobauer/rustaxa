#!/usr/bin/env python3
"""Run exact C++ query methods against recording leaves; no state/RPC parity claim."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
HERE = pathlib.Path(__file__).resolve().parent
SOURCE = ROOT / 'libraries/core_libs/consensus/src/application/external_evm_state_owner.cpp'
FIXTURE = HERE / 'fixtures/query_policy_reference.json'

def generate():
    source = SOURCE.read_text()
    start = source.index('std::optional<state_api::Account> ExternalEvmStateOwner::account(')
    end = source.index('h256 ExternalEvmStateOwner::readBridgeContractHash(', start)
    methods = source[start:end]
    template = (HERE / 'query_policy_reference.cpp').read_text()
    if template.count('// EXTRACTED_METHODS') != 1:
        raise RuntimeError('ambiguous extraction insertion')
    with tempfile.TemporaryDirectory(prefix='rustaxa-query-policy-') as directory:
        path = pathlib.Path(directory)
        cpp = path / 'reference.cpp'
        binary = path / 'reference'
        cpp.write_text(template.replace('// EXTRACTED_METHODS', methods))
        subprocess.run(['c++', '-std=c++20', '-O2', str(cpp), '-o', str(binary)], check=True)
        output = subprocess.check_output([str(binary)], text=True, timeout=10)
    return {'source': str(SOURCE.relative_to(ROOT)),
            'methods_sha256': hashlib.sha256(methods.encode()).hexdigest(),
            'harness_sha256': hashlib.sha256(template.encode()).hexdigest(),
            'scope': 'selection only; stable heads, readable owner and available headers; leaves stop before state execution',
            'cases': [json.loads(line) for line in output.splitlines()]}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    result = generate()
    if args.record:
        FIXTURE.write_text(json.dumps(result, indent=2) + '\n')
    elif result != json.loads(FIXTURE.read_text()):
        raise RuntimeError('query policy fixture mismatch')
    print(f"verified {len(result['cases'])} C++ query-selection cases")

if __name__ == '__main__':
    main()
