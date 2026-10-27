#!/usr/bin/env python3
"""Execute the unchanged C++ gas search on every retained era Go probe."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import tempfile

from mainnet_era_profile import ROOT
from native_integration_reference import capture_child

HERE = Path(__file__).resolve().parent
CORPUS = HERE / 'fixtures/mainnet_era'
CPP = ROOT / 'libraries/core_libs/network/rpc/eth/Eth.cpp'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def load(pin):
    raw = (CORPUS / pin['file']).read_bytes()
    if sha(raw) != pin['gzip_sha256'] or len(raw) != pin['gzip_bytes']:
        raise ValueError('compressed stream identity mismatch')
    body = gzip.decompress(raw)
    if sha(body) != pin['sha256'] or len(body) != pin['bytes']:
        raise ValueError('inner stream identity mismatch')
    return json.loads(body)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scenario', required=True)
    args = parser.parse_args()
    manifest_path = CORPUS / (args.scenario + '.manifest.json')
    original = manifest_path.read_bytes()
    manifest = json.loads(original)
    documents = [(label, load(pin)) for label, pin in manifest['streams'].items()]
    for label, forks in manifest['pre_reward_forks'].items():
        documents.extend((label + '/pre_reward/' + period, load(pin)) for period, pin in forks.items())
    cases = []
    for label, document in documents:
        for case in document['api_cases']:
            cases.extend((label, estimate) for estimate in case['estimates'])
    if not cases:
        raise ValueError('zero matched gas-search targets')
    source = CPP.read_text()
    method = source.split('string eth_estimateGas(', 1)[1]
    body = method[method.index('    auto is_enough_gas'):]
    body = body[:body.index('    return toJS(hi);') + len('    return toJS(hi);')]
    prefix = r'''
#include <cstdint>
#include <iostream>
#include <iomanip>
#include <optional>
#include <stdexcept>
#include <string>
#include <vector>
using gas_t = uint64_t;
struct Result { std::string consensus_err, code_err; gas_t gas_used; };
struct Row { gas_t gas; Result result; };
struct Transaction { std::optional<gas_t> gas; };
gas_t toJS(gas_t value) { return value; }
int main() {
'''
    chunks = [prefix]
    for _, case in cases:
        rows = ','.join('{' + str(p['gas']) + ',{' + json.dumps(p['output']['consensus_error']) + ',' +
                        json.dumps(p['output']['execution_error']) + ',' + str(p['output']['gas_used']) + '}}'
                        for p in case['probes'])
        chunks.append('{' + f'Transaction t{{{case["cap"]}}}; int blk_n=0; size_t index=0; std::vector<Row> rows{{{rows}}};' + r'''
auto call = [&](int, Transaction request) -> Result {
  if (index >= rows.size() || *request.gas != rows[index].gas) throw std::runtime_error("probe transcript mismatch");
  return rows[index++].result;
};
auto estimate = [&]() -> gas_t {
''' + body + r'''
};
std::cout << "{";
try { auto value=estimate(); std::cout << "\"result\":" << value; }
catch (const std::exception& error) { std::cout << "\"error\":" << std::quoted(error.what()); }
std::cout << ",\"consumed\":" << index << "}\n";
if(index != rows.size()) return 2;
}
''')
    chunks.append('}\n')
    directory = Path(tempfile.mkdtemp(prefix='era-cpp-estimate-', dir=os.environ['RUN_ARTIFACT_DIR']))
    effective = directory / 'reference.cpp'
    effective.write_text(''.join(chunks))
    compile_dir = directory / 'compile'
    compile_dir.mkdir()
    compile_status = capture_child(['rtk', 'proxy', 'c++', '-v', '-std=c++20', '-O0', str(effective), '-o', str(directory / 'reference')], ROOT, compile_dir, timeout_seconds=120)
    if not compile_status['capture_complete'] or compile_status['child_exit']:
        raise RuntimeError('C++ gas search compilation failed')
    execute_dir = directory / 'execute'
    execute_dir.mkdir()
    status = capture_child(['rtk', 'proxy', str(directory / 'reference')], ROOT, execute_dir, timeout_seconds=120)
    if not status['capture_complete'] or status['child_exit']:
        raise RuntimeError('unchanged C++ gas search rejected a Go probe transcript')
    lines = gzip.decompress((execute_dir / 'stdout.gz').read_bytes()).splitlines()
    if len(lines) != len(cases):
        raise ValueError('missing C++ gas-search results')
    results = []
    for (label, case), line in zip(cases, lines, strict=True):
        result = json.loads(line)
        if result['consumed'] != len(case['probes']) or result.get('error') == 'probe transcript mismatch':
            raise ValueError('C++ gas-search transcript mismatch')
        results.append({'source': label, 'name': case['name'], **result})
    result = {'schema': 1, 'algorithm_sha256': sha(body.encode()), 'cpp_source_sha256': sha(CPP.read_bytes()),
              'effective_source_sha256': sha(effective.read_bytes()), 'input_manifest_sha256': sha(original),
              'capture': str(directory), 'cases': results}
    raw = (json.dumps(result, sort_keys=True, separators=(',', ':')) + '\n').encode()
    target = CORPUS / (sha(raw) + '.cpp-estimates.json')
    if not target.exists():
        target.write_bytes(raw)
    history = CORPUS / (sha(original) + '.manifest.json')
    if not history.exists():
        history.write_bytes(original)
    manifest['cpp_estimates'] = {'file': target.name, 'sha256': sha(raw), 'bytes': len(raw)}
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    print('Actual unchanged C++ search:', len(results), 'cases,', sum(c['consumed'] for c in results), 'probes')


if __name__ == '__main__':
    main()
