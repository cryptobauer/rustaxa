#!/usr/bin/env python3
"""Compare actual metadata DryRunner probes with the unchanged C++ gas search."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = Path(__file__).resolve().parent
SOURCES = [HERE / 'native_simulation_reference.go', HERE / 'native_metadata_estimate_reference.go']
INPUT = HERE / 'fixtures/native_metadata_simulation/public.json'
FIXTURES = HERE / 'fixtures/native_metadata_estimate'
CPP_SOURCE = ROOT / 'libraries/core_libs/network/rpc/eth/Eth.cpp'


def run_reference(revision, requests):
    with tempfile.TemporaryDirectory(prefix='rustaxa-metadata-estimate-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        command = Path(directory) / 'cmd/metadata_estimate_reference'
        command.mkdir(parents=True)
        for index, source in enumerate(SOURCES):
            content = source.read_text()
            if index == 0:
                if content.count('func main() {') != 1:
                    raise RuntimeError('Shared main shape changed')
                content = content.replace('func main() {', 'func unusedSeedMain() {')
            (command / source.name).write_text(content)
        return subprocess.check_output(['go', 'run', '-mod=readonly', './cmd/metadata_estimate_reference'], cwd=directory, input=json.dumps(requests).encode())


def cpp_reference(document):
    method = CPP_SOURCE.read_text().split('string eth_estimateGas(', 1)[1]
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
    for case in document['cases']:
        rows = ','.join('{' + str(probe['gas']) + ',{' + json.dumps(probe['output']['consensus_error']) + ',' + json.dumps(probe['output']['execution_error']) + ',' + str(probe['output']['gas_used']) + '}}' for probe in case['probes'])
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
    with tempfile.TemporaryDirectory(prefix='rustaxa-metadata-estimate-cpp-') as directory:
        path = Path(directory)
        (path / 'reference.cpp').write_text(''.join(chunks))
        subprocess.run(['c++', '-std=c++20', '-O2', str(path / 'reference.cpp'), '-o', str(path / 'reference')], check=True)
        lines = subprocess.check_output([str(path / 'reference')], timeout=10).decode().splitlines()
    cases = []
    for case, line in zip(document['cases'], lines, strict=True):
        output = json.loads(line)
        if output.get('error') == 'probe transcript mismatch':
            raise RuntimeError('Actual C++ search rejected candidate transcript')
        cases.append({'name': case['name'], **output})
    return {'algorithm_sha256': hashlib.sha256(body.encode()).hexdigest(), 'cases': cases}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    requests = json.loads(INPUT.read_bytes())['cases']
    outputs = {label: run_reference(revision, requests) for label, revision in REVISIONS.items()}
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Pinned estimation probes differ')
    documents = [json.loads(data) for data in outputs.values()]
    for document in documents:
        if document['schema'] != 1 or document['state_before'] != document['state_after'] or len(document['cases']) != 9:
            raise RuntimeError('Invalid estimation corpus')
    cpp = cpp_reference(documents[0])
    cpp_bytes = (json.dumps(cpp, sort_keys=True, indent=2) + '\n').encode()
    outputs['cpp'] = cpp_bytes
    manifest = {'schema': 1, 'references': REVISIONS,
                'source_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in SOURCES},
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
            raise RuntimeError('Estimate manifest changed')
        for label, data in outputs.items():
            if data != (FIXTURES / f'{label}.json').read_bytes():
                raise RuntimeError(f'{label}: estimate fixture changed')
    print('Both actual Go probe corpora matched the unchanged C++ search')


if __name__ == '__main__':
    main()
