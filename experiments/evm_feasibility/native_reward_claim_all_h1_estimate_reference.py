#!/usr/bin/env python3
"""Record/reproduce funded H1 claim-all estimate probes and unchanged C++ search."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from reference import ROOT,REVISIONS
import native_reward_claim_all_h1_reference as shared
HERE=Path(__file__).resolve().parent
SOURCE=HERE/'native_reward_claim_all_h1_estimate_reference.go'
FIXTURES=HERE/'fixtures/native_reward_claim_all_h1_estimate'
CPP_SOURCE=ROOT/'libraries/core_libs/network/rpc/eth/Eth.cpp'
SUPPORT=[*shared.SUPPORT,HERE/'native_reward_claim_all_h1_reference.go']
shared.SOURCE=SOURCE
shared.SUPPORT=SUPPORT
def digest(data):return hashlib.sha256(data).hexdigest()
def strip(value):
 if isinstance(value,dict):return {k:strip(v) for k,v in value.items() if k!='observation_witness'}
 if isinstance(value,list):return [strip(v) for v in value]
 return value
def validate(document):
 baseline=json.loads((HERE/'fixtures/native_reward_claim_all_h1/public.json').read_text())
 direct=baseline['cases'][0]
 assert document['schema']==1 and document['repeat_identical'] and document['go_native_period']==document['go_evm_block']==1
 assert document['state_before']==document['state_after']==baseline['state_before']
 assert [(c['name'],c['cap']) for c in document['cases']]==[('claim_all_H1_cap200000',200000),('claim_all_H1_minimum',111272),('claim_all_H1_minimum_plus_one',111273)]
 assert [[p['gas'] for p in c['probes']] for c in document['cases']]==[[200000,155636,133454,122363,116817],[111272],[111273]]
 for case in document['cases']:
  for p in case['probes']:
   assert p['name']==case['name'] and p['gas']>=111272
   assert p['input']=='0b83a727' and p['to']==direct['to'] and p['supplied_nonce']==str(2**512) and p['gas_price']==p['value']=='0'
   assert p['output']==direct['output']
   assert p['quotes']==[{**direct['quotes'][0],'supplied_native_gas':p['gas']-21272}]
   assert p['observation_witness']=={'raw':direct['ordered_raw_writes'],'balances':direct['ordered_balance_effects']}
   for field in ['accounts','prior_raw','after_raw']:assert p[field]==direct[field]

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
        subprocess.run(['rtk','proxy','c++', '-std=c++20', '-O2', str(path / 'reference.cpp'), '-o', str(path / 'reference')], check=True)
        lines = subprocess.check_output(['rtk','proxy',str(path / 'reference')], timeout=10).decode().splitlines()
    cases = []
    for case, line in zip(document['cases'], lines, strict=True):
        output = json.loads(line)
        if output.get('error') == 'probe transcript mismatch':
            raise RuntimeError('Actual C++ search rejected candidate transcript')
        cases.append({'name': case['name'], **output})
    return {'algorithm_sha256': hashlib.sha256(body.encode()).hexdigest(), 'cases': cases}


def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--record',action='store_true');args=parser.parse_args()
 outputs={label:shared.run_reference(rev) for label,rev in REVISIONS.items()}
 controls={label:shared.run_reference(rev,True) for label,rev in REVISIONS.items()}
 assert outputs['public']==outputs['local'] and controls['public']==controls['local']
 for label in REVISIONS:
  document=json.loads(outputs[label][0]);validate(document)
  assert strip(document)==strip(json.loads(controls[label][0]))
 cpp=cpp_reference(json.loads(outputs['public'][0]))
 assert cpp['algorithm_sha256']=='7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759'
 assert cpp['cases']==[{'name':'claim_all_H1_cap200000','result':116817,'consumed':5},{'name':'claim_all_H1_minimum','result':111272,'consumed':1},{'name':'claim_all_H1_minimum_plus_one','result':111273,'consumed':1}]
 files={'cpp.json':(json.dumps(cpp,indent=2,sort_keys=True)+'\n').encode()}
 for label,data in {**outputs,**{k+'.control':v for k,v in controls.items()}}.items():
  for suffix,value in zip(['json','stdout.txt','stderr.txt'],data,strict=True):files[label+'.'+suffix]=value
 manifest={'schema':1,'references':REVISIONS,'profile':'funded_only_HistoricalSimulation1_positive_claim_all_estimate','go_version':subprocess.check_output(['rtk','proxy','go','version'],text=True).strip(),'exporter_sha256':digest(SOURCE.read_bytes()),'harness_sha256':digest(Path(__file__).read_bytes()),'support_sha256':{p.name:digest(p.read_bytes()) for p in SUPPORT},'shared_harness_sha256':digest(Path(shared.__file__).read_bytes()),'accepted_h1_fixture_sha256':digest((HERE/'fixtures/native_reward_claim_all_h1/public.json').read_bytes()),'cpp_source_sha256':digest(CPP_SOURCE.read_bytes()),'cpp_helper_source_sha256':digest((HERE/'native_metadata_estimate_reference.py').read_bytes()),'algorithm_sha256':cpp['algorithm_sha256'],'copy_observers_in_both_runs':True,'copy_observers':{'dry_source_sha256':shared.DRY_SHA,'dry_needle_sha256':digest(shared.DRY_NEEDLE),'dry_helper_sha256':digest(shared.POST_HELPER),'vm_source_sha256':shared.VM_SHA,'vm_needle_sha256':digest(shared.VM_NEEDLE),'vm_helper_sha256':digest(shared.QUOTE_HELPER)},'raw_balance_guard_source_sha256':digest(Path(shared.custody.__file__).read_bytes()),'raw_helper_sha256':digest(shared.custody.TRACE_HELPER),'balance_helper_sha256':digest(shared.reward.BALANCE_HELPER),'balance_helper_source_sha256':digest(Path(shared.reward.__file__).read_bytes()),'sha256':{k:digest(v) for k,v in files.items()}}
 files['manifest.json']=(json.dumps(manifest,indent=2,sort_keys=True)+'\n').encode()
 if args.record:FIXTURES.mkdir(parents=True,exist_ok=True)
 for name,value in files.items():
  p=FIXTURES/name
  if args.record:p.write_bytes(value)
  else:assert p.read_bytes()==value,str(p)
 print('Actual funded H1 claim-all dualpin probes and controls matched unchanged C++ search')
if __name__=='__main__':main()
