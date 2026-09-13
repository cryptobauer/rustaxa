#!/usr/bin/env python3
"""Extract and run the existing C++ gas search without changing its algorithm.

The finite callback scenarios isolate Eth.cpp's search/error policy. They do
not establish EVM or RPC routing parity. The impossible one-unit looping case
is intentionally excluded; Rust reports that invalid-probe condition explicitly.
"""

import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE = ROOT / "libraries/core_libs/network/rpc/eth/Eth.cpp"
FIXTURE = pathlib.Path(__file__).resolve().parent / "fixtures/estimate_reference.json"


def generate():
    """Return exact source identity and C++-produced probe transcripts."""
    source = SOURCE.read_text()
    method = source.split("string eth_estimateGas(", 1)[1]
    body = method[method.index("    auto is_enough_gas"):]
    body = body[:body.index("    return toJS(hi);") + len("    return toJS(hi);")]
    prefix = r'''
#include <cstdint>
#include <iostream>
#include <limits>
#include <optional>
#include <stdexcept>
#include <string>
#include <vector>
using gas_t = uint64_t;
struct Result { std::string consensus_err, code_err; gas_t gas_used; };
struct Transaction { std::optional<gas_t> gas; };
gas_t toJS(gas_t value) { return value; }
int main() {
  for (int scenario=0; scenario<8; ++scenario) {
    gas_t cap = scenario==0 ? 100 : scenario==1 ? 0 : scenario==2 ? 1 :
                scenario==3 ? std::numeric_limits<gas_t>::max() : 100;
    Transaction t{cap};
    int blk_n = 0;
    std::vector<gas_t> probes;
    auto call = [&](int, Transaction request) -> Result {
      auto gas = *request.gas;
      probes.push_back(gas);
      if (scenario==0) return {"", gas<61 ? "execution reverted" : "", 61};
      if (scenario==1 || scenario==2) return {"", "", 0};
      if (scenario==3) return {"", "", std::numeric_limits<gas_t>::max()-1};
      if (scenario==4) return {"nonce too low", "", 0};
      if (scenario==5) return {"", "execution reverted: denied", 0};
      if (scenario==6) return {gas==70 ? "future block" : "", "", 40};
      return {"", gas<60 ? "out of gas" : "", 1};
    };
    auto estimate = [&]() -> gas_t {
'''
    suffix = r'''
    };
    std::cout << "{\"scenario\":" << scenario;
    try { auto result = estimate(); std::cout << ",\"result\":" << result; }
    catch (const std::exception& error) {
      std::cout << ",\"error\":\"" << error.what() << "\"";
    }
    std::cout << ",\"probes\":[";
    for (size_t i=0; i<probes.size(); ++i) {
      if (i) std::cout << ',';
      std::cout << probes[i];
    }
    std::cout << "]}\n";
  }
}
'''
    with tempfile.TemporaryDirectory(prefix="rustaxa-estimate-reference-") as directory:
        path = pathlib.Path(directory)
        cpp = path / "reference.cpp"
        binary = path / "reference"
        cpp.write_text(prefix + body + suffix)
        subprocess.run(["c++", "-std=c++20", "-O2", str(cpp), "-o", str(binary)], check=True)
        output = subprocess.check_output([str(binary)], text=True, timeout=10)
    return {
        "source": str(SOURCE.relative_to(ROOT)),
        "algorithm_sha256": hashlib.sha256(body.encode()).hexdigest(),
        "cases": [json.loads(line) for line in output.splitlines()],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    result = generate()
    if args.record:
        FIXTURE.write_text(json.dumps(result, indent=2) + "\n")
    elif result != json.loads(FIXTURE.read_text()):
        raise SystemExit("C++ estimator reference differs from checked-in fixture")
    print(f"verified {len(result['cases'])} C++ gas-search scenarios")


if __name__ == "__main__":
    main()
