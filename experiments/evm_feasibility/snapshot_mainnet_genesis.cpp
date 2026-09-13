#include <iostream>

#include "cli/config.hpp"
#include "cli/tools.hpp"
#include "config/genesis.hpp"

int main() {
  taraxa::GenesisConfig genesis;
  taraxa::dec_json(taraxa::cli::tools::getGenesis(taraxa::cli::Config::ChainIdType::Mainnet), genesis);
  std::cout << genesis.genesisHash() << '\n';
}
