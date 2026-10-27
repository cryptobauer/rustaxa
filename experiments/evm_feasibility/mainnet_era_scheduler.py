#!/usr/bin/env python3
"""Extract the unchanged C++ reward scheduler for finite synthetic era inputs.

Only external database/PBFT leaves are replaced. The complete processStats,
clear, recoverFromDb and getBlockStats method bodies are retained. BlockStats
payload construction is a recording leaf here; aggregation has its own oracle.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path

from mainnet_era_profile import ROOT, profile, scenarios
from native_integration_reference import capture_child

SOURCE = ROOT / 'libraries/core_libs/consensus/src/rewards/rewards_stats.cpp'
HEADER = ROOT / 'libraries/core_libs/consensus/include/rewards/rewards_stats.hpp'
STORAGE = ROOT / 'libraries/core_libs/storage/src/storage.cpp'

PREFIX = r'''
#include <algorithm>
#include <array>
#include <bit>
#include <cstring>
#include <limits>
#include <cstdint>
#include <functional>
#include <iostream>
#include <iterator>
#include <map>
#include <memory>
#include <unordered_map>
#include <vector>
using EthBlockNumber = uint64_t;
using PbftPeriod = uint64_t;
using gas_t = uint64_t;
using Batch = int;
struct Pbft { uint64_t period; uint64_t getPeriod() const { return period; } };
struct Vote { uint64_t period; uint64_t getPeriod() const { return period; } };
struct PeriodData { std::shared_ptr<Pbft> pbft_blk; std::vector<std::shared_ptr<Vote>> previous_block_cert_votes; };
struct HardforksConfig {
  struct { uint64_t block_num; } magnolia_hf;
  struct { uint64_t block_num_part_one; } aspen_hf;
  uint64_t frequency_start;
  uint32_t getRewardsDistributionFrequency(uint64_t p) const { return p < frequency_start ? 1 : 100; }
  bool isAspenHardforkPartOne(uint64_t p) const { return p >= aspen_hf.block_num_part_one; }
};
namespace taraxa::rewards {
struct BlockStats {
  uint64_t period=0; uint32_t rate=0;
  BlockStats()=default;
  BlockStats(const PeriodData& b, uint32_t r, const std::vector<gas_t>&, uint64_t, uint32_t, bool=false)
      : period(b.pbft_blk->period), rate(r) {}
};
struct DbStorage {
  enum class Columns { block_rewards_stats };
  // Source: storage.cpp:DbStorage::getBlocksRewardsStats, pinned in the
  // manifest. Its RocksDB iterator visits bytewise native-PbftPeriod keys and
  // inserts decoded rows into a fresh unordered_map; copying a cached map
  // would preserve the wrong bucket/insertion history across legacy recovery.
  using Key = std::array<unsigned char,sizeof(PbftPeriod)>;
  std::map<Key,std::pair<PbftPeriod,BlockStats>> rows;
  auto getBlocksRewardsStats() {
    std::unordered_map<PbftPeriod,BlockStats> result;
    for (const auto& [key,row] : rows) result[row.first] = row.second;
    return result;
  }
  void deleteColumnData(Columns) { rows.clear(); }
  void saveBlockRewardsStats(uint64_t p, const BlockStats& b, Batch&) {
    Key key; std::memcpy(key.data(),&p,sizeof(p)); rows[key] = {p,b};
  }
};
class Stats {
 public:
  Stats(uint32_t, const HardforksConfig&, std::shared_ptr<DbStorage>, std::function<uint64_t(EthBlockNumber)>&&, EthBlockNumber=0);
  void recoverFromDb(EthBlockNumber);
  void clear(uint64_t);
  BlockStats getBlockStats(const PeriodData&, uint32_t, const std::vector<gas_t>&);
  std::vector<BlockStats> processStats(const PeriodData&, uint32_t, const std::vector<gas_t>&, Batch&);
  const uint32_t kCommitteeSize;
  const HardforksConfig kHardforksConfig;
  std::shared_ptr<DbStorage> db_;
  const std::function<uint64_t(EthBlockNumber)> dpos_eligible_total_vote_count_;
  std::unordered_map<PbftPeriod, BlockStats> blocks_stats_;
};
}
'''

SUFFIX = r'''
int main() {
 using namespace taraxa::rewards;
 static_assert(sizeof(PbftPeriod)==8 && std::numeric_limits<PbftPeriod>::digits==64);
 std::cerr << "compiler=" << __VERSION__ << " cplusplus=" << __cplusplus
           << " period_key_endian=" << (std::endian::native==std::endian::little ? "little" : "big");
#ifdef __GLIBCXX__
 std::cerr << " libstdcxx=" << __GLIBCXX__;
#endif
#ifdef _GLIBCXX_RELEASE
 std::cerr << " libstdcxx_release=" << _GLIBCXX_RELEASE;
#endif
 std::cerr << "\n";
 uint64_t start, end, reopen; uint32_t rate;
 while (std::cin >> start >> end >> rate >> reopen) {
  HardforksConfig hf{{5730000},{8118000},5730000};
  auto db=std::make_shared<DbStorage>();
  auto make=[&](uint64_t head) { return std::make_unique<Stats>(1000,hf,db,[](uint64_t){return uint64_t(3000);},head); };
  // A declared reopen before the first supplied row starts from the actual
  // empty DB input leaf; no missing row is fabricated to reach that coordinate.
  auto stats=make(reopen < start ? reopen : 0); Batch batch=0;
  for (auto p=start; p<=end; ++p) {
   PeriodData b{std::make_shared<Pbft>(Pbft{p}), {}};
   auto result=stats->processStats(b,rate,{},batch);
   std::cout << "{\"period\":" << p << ",\"order\":[";
   bool first=true;
   for (const auto& row:result) { if(!first) std::cout << ','; first=false; std::cout << row.period; }
   std::cout << "]}\n";
   stats->clear(p);
   if (p==reopen) stats=make(p);
  }
 }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', default='cpp-scheduler', help='Exclusive new child directory in RUN_ARTIFACT_DIR')
    args = parser.parse_args()
    if re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*', args.directory) is None:
        raise ValueError('safe new directory basename required')
    run = Path(os.environ['RUN_ARTIFACT_DIR'])
    directory = run / args.directory
    directory.mkdir(exist_ok=False)
    raw = SOURCE.read_bytes()
    text = raw.decode()
    # Remove only include directives; all definitions, namespaces and bodies
    # remain byte-identical. Retain both the original and effective source.
    effective = PREFIX + '\n'.join(line for line in text.splitlines() if not line.startswith('#include')) + SUFFIX
    (directory / 'original.cpp').write_bytes(raw)
    (directory / 'original.hpp').write_bytes(HEADER.read_bytes())
    storage_raw = STORAGE.read_bytes()
    (directory / 'original-storage.cpp').write_bytes(storage_raw)
    signature = 'std::unordered_map<PbftPeriod, rewards::BlockStats> DbStorage::getBlocksRewardsStats() const'
    storage_text = storage_raw.decode()
    if storage_text.count(signature) != 1:
        raise ValueError('actual storage recovery method extraction guard')
    method_start = storage_text.index(signature)
    brace = storage_text.index('{', method_start)
    level, method_end = 1, brace + 1
    while level:
        if method_end >= len(storage_text):
            raise ValueError('unterminated actual storage recovery method')
        level += (storage_text[method_end] == '{') - (storage_text[method_end] == '}')
        method_end += 1
    storage_method = storage_text[method_start:method_end].encode()
    (directory / 'original-storage-method.cpp').write_bytes(storage_method)
    (directory / Path(__file__).name).write_bytes(Path(__file__).read_bytes())
    source = directory / 'scheduler.cpp'
    source.write_text(effective)
    compiler = capture_child(['rtk', 'proxy', 'c++', '-v', '-std=c++20', '-O0', str(source), '-o', str(directory / 'scheduler')],
                             ROOT, directory, timeout_seconds=120)
    if not compiler['capture_complete'] or compiler['child_exit']:
        raise RuntimeError('C++ scheduler compilation failed')
    # The current capture helper has closed stdin. Keep the input in a source
    # wrapper which opens this exact retained file rather than a shell pipe.
    cases = []
    policy = profile()
    for scenario in scenarios():
        first = (scenario['pending_periods'] or [scenario['periods'][0]])[0]
        rate = policy['blocks_per_year_before_cacti']
        if first >= 24350801:
            rate = 365 * 24 * 60 * 60 * 1000 // (2 * 1000 + 400)
        modes = list(dict.fromkeys([0, scenario['head'], scenario['first_retained']]))
        if not first <= scenario['periods'][-1] or any(type(p) is not int or p < 0 or p > scenario['periods'][-1] for p in modes):
            raise ValueError('invalid finite scheduler/reopen coordinates')
        for reopen in modes:
            cases.append({'name': scenario['name'], 'start': first,
                          'end': scenario['periods'][-1], 'rate': rate, 'reopen': reopen})
    input_file = directory / 'input.txt'
    input_file.write_text(''.join(f"{c['start']} {c['end']} {c['rate']} {c['reopen']}\n" for c in cases))
    wrapper = directory / 'run.py'
    wrapper.write_text('import subprocess\nfrom pathlib import Path\np=Path(__file__).parent\nwith (p/"input.txt").open("rb") as f:\n subprocess.run(["rtk","proxy",str(p/"scheduler")],stdin=f,check=True)\n')
    execution = directory / 'execution'
    execution.mkdir()
    result = capture_child(['rtk', 'proxy', 'python3', str(wrapper)], ROOT, execution, timeout_seconds=120)
    if not result['capture_complete'] or result['child_exit']:
        raise RuntimeError('C++ scheduler execution failed')
    import gzip
    lines = [json.loads(line) for line in gzip.decompress((execution / 'stdout.gz').read_bytes()).splitlines()]
    outputs = []
    offset = 0
    for case in cases:
        size = case['end'] - case['start'] + 1
        rows = lines[offset:offset + size]
        offset += size
        if [r['period'] for r in rows] != list(range(case['start'], case['end'] + 1)):
            raise ValueError('incomplete C++ scheduler output')
        outputs.append({**case, 'rows': rows})
    if offset != len(lines):
        raise ValueError('unconsumed C++ scheduler output')
    pin = lambda path: {'bytes': path.stat().st_size, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
    manifest = {'schema': 1, 'scope': 'actual C++ scheduling and exact period order; BlockStats payload is a recording leaf',
                'sources': {str(p.relative_to(ROOT)): pin(p) for p in [SOURCE, HEADER, STORAGE, Path(__file__).resolve()]},
                'storage_recovery_leaf': {'source_method':signature,
                    'source_method_sha256':hashlib.sha256(storage_method).hexdigest(),
                    'source_file_sha256':hashlib.sha256(storage_raw).hexdigest(),
                    'key_order':'RocksDB bytewise iteration of native uint64 period bytes, then insertion into fresh std::unordered_map',
                    'scope':'typed database input leaf, not a storage codec/physical-DB proof'},
                'effective_source': pin(source), 'inputs': pin(input_file),
                'standard_library': 'compiler -v capture and runtime compiler/libstdc++/native-key-endian metadata in execution stderr.gz',
                'cases': outputs}
    (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print('Captured', len(outputs), 'C++ schedule cases;', len(lines), 'period rows')


if __name__ == '__main__':
    main()
