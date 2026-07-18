#include "final_chain/final_chain.hpp"

#include <libdevcore/CommonData.h>

#include <array>
#include <limits>
#include <optional>
#include <vector>

#include "common/constants.hpp"
#include "common/encoding_rlp.hpp"
#include "common/encoding_solidity.hpp"
#include "common/vrf_wrapper.hpp"
#include "config/config.hpp"
#include "final_chain/trie_common.hpp"
#include "libdevcore/CommonJS.h"
#include "network/rpc/eth/Eth.h"
#include "test_util/gtest.hpp"
#include "test_util/samples.hpp"
#include "test_util/test_util.hpp"
#include "vote/pbft_vote.hpp"

namespace taraxa::final_chain {
using namespace taraxa::core_tests;

struct advance_check_opts {
  bool dont_assume_no_logs = 0;
  bool dont_assume_all_trx_success = 0;
  bool expect_to_fail = 0;
};

struct FinalChainTest : WithDataDir {
  std::shared_ptr<DbStorage> db{new DbStorage(data_dir / "db")};
  FullNodeConfig cfg = FullNodeConfig();
  std::shared_ptr<final_chain::FinalChain> SUT;
  bool assume_only_toplevel_transfers = true;
  std::unordered_map<addr_t, u256> expected_balances;
  uint64_t expected_blk_num = 0;
  dev::KeyPair dag_proposer_keys = dev::KeyPair::create();
  dev::KeyPair pbft_proposer_keys = dev::KeyPair::create();

  void create_validators() {
    dev::KeyPair validator_owner_keys = dev::KeyPair::create();
    cfg.genesis.state.initial_balances[validator_owner_keys.address()] =
        10 * cfg.genesis.state.dpos.validator_maximum_stake;
    for (const auto& keys : {dag_proposer_keys, pbft_proposer_keys}) {
      const auto vrf_pub_key = taraxa::vrf_wrapper::getVrfKeyPair().first;
      state_api::ValidatorInfo validator{keys.address(), validator_owner_keys.address(), vrf_pub_key, 0, "", "", {}};
      validator.delegations.emplace(validator_owner_keys.address(), cfg.genesis.state.dpos.validator_maximum_stake);
      cfg.genesis.state.dpos.initial_validators.emplace_back(validator);
    }
  }

  void init() {
    SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
    const auto& effective_balances = effective_initial_balances(cfg.genesis.state);
    cfg.genesis.state.dpos.yield_percentage = 0;
    for (const auto& [addr, _] : cfg.genesis.state.initial_balances) {
      auto acc_actual = SUT->getAccount(addr);
      ASSERT_TRUE(acc_actual);
      const auto expected_bal = effective_balances.at(addr);
      ASSERT_EQ(acc_actual->balance, expected_bal);
      expected_balances[addr] = expected_bal;
    }
  }

  auto advance(const SharedTransactions& trxs, advance_check_opts opts = {}) {
    std::vector<h256> trx_hashes;
    ++expected_blk_num;
    for (const auto& trx : trxs) {
      trx_hashes.emplace_back(trx->getHash());
    }

    auto dag_blk = std::make_shared<DagBlock>(blk_hash_t{}, level_t{}, vec_blk_t{}, trx_hashes, 0, VdfSortition{},
                                              dag_proposer_keys.secret());
    db->saveDagBlock(dag_blk);
    std::vector<vote_hash_t> reward_votes_hashes;
    auto pbft_block = std::make_shared<PbftBlock>(
        kNullBlockHash, kNullBlockHash, kNullBlockHash, kNullBlockHash, expected_blk_num, addr_t::random(),
        pbft_proposer_keys.secret(), reward_votes_hashes, PbftBlockExtraData(1, 0, 0, 1, "", blk_hash_t(123)));

    std::vector<std::shared_ptr<PbftVote>> votes;
    PeriodData period_data(pbft_block, votes);
    period_data.dag_blocks.push_back(dag_blk);
    period_data.transactions = trxs;
    if (pbft_block->getPeriod() > 1) {
      period_data.previous_block_cert_votes = {
          genDummyVote(PbftVoteTypes::cert_vote, pbft_block->getPeriod() - 1, 1, 3, pbft_block->getBlockHash())};
    }

    auto batch = db->createWriteBatch();
    db->savePeriodData(period_data, batch);
    db->commitWriteBatch(batch);

    auto result =
        SUT->finalize(std::move(period_data), {dag_blk->getHash()}, cfg.genesis.state.dpos.blocks_per_year).get();
    const auto& blk_h = *result->final_chain_blk;
    EXPECT_EQ(util::rlp_enc(blk_h), util::rlp_enc(*SUT->blockHeader(blk_h.number)));
    EXPECT_EQ(util::rlp_enc(blk_h), util::rlp_enc(*SUT->blockHeader()));
    const auto& receipts = result->trx_receipts;
    EXPECT_EQ(blk_h.hash, SUT->blockHeader()->hash);
    EXPECT_EQ(blk_h.hash, SUT->blockHash());
    EXPECT_EQ(blk_h.parent_hash, SUT->blockHeader(expected_blk_num - 1)->hash);
    EXPECT_EQ(blk_h.number, expected_blk_num);
    EXPECT_EQ(blk_h.number, SUT->lastBlockNumber());
    EXPECT_EQ(SUT->transactionCount(blk_h.number), trxs.size());
    for (size_t i = 0; i < trxs.size(); i++) EXPECT_EQ(*SUT->transactions(blk_h.number)[i], *trxs[i]);
    EXPECT_EQ(*SUT->blockNumber(*SUT->blockHash(blk_h.number)), expected_blk_num);
    EXPECT_EQ(blk_h.author, pbft_block->getBeneficiary());
    EXPECT_EQ(blk_h.timestamp, pbft_block->getTimestamp());
    EXPECT_EQ(receipts.size(), trxs.size());
    EXPECT_EQ(blk_h.transactions_root,
              trieRootOver(trxs.size(), [&](auto i) { return dev::rlp(i); }, [&](auto i) { return trxs[i]->rlp(); }));
    EXPECT_EQ(blk_h.receipts_root, trieRootOver(
                                       trxs.size(), [&](auto i) { return dev::rlp(i); },
                                       [&](auto i) { return util::rlp_enc(receipts[i]); }));
    EXPECT_EQ(blk_h.gas_limit, cfg.genesis.pbft.gas_limit);
    EXPECT_EQ(blk_h.extra_data, pbft_block->getExtraDataRlp());
    EXPECT_EQ(blk_h.nonce(), Nonce());
    EXPECT_EQ(blk_h.difficulty(), 0);
    EXPECT_EQ(blk_h.mixHash(), h256());
    EXPECT_EQ(blk_h.unclesHash(), EmptyRLPListSHA3());
    EXPECT_TRUE(!blk_h.state_root.isZero());
    LogBloom expected_block_log_bloom;
    std::unordered_set<addr_t> all_addrs_w_changed_balance;
    uint64_t cumulative_gas_used_actual = 0;
    for (size_t i = 0; i < trxs.size(); ++i) {
      const auto& trx = trxs[i];
      const auto& r = receipts[i];
      if (!opts.expect_to_fail) {
        EXPECT_TRUE(r.gas_used != 0);
      }
      EXPECT_EQ(util::rlp_enc(r), util::rlp_enc(*SUT->transactionReceipt(blk_h.number, i)));
      cumulative_gas_used_actual += r.gas_used;
      if (assume_only_toplevel_transfers) {
        const auto& sender = trx->getSender();
        expected_balances[sender] -= r.gas_used * trx->getGasPrice();
        all_addrs_w_changed_balance.insert(sender);
        const auto& receiver = !trx->getReceiver() ? *r.new_contract_address : *trx->getReceiver();
        if (r.status_code == 1) {
          expected_balances[sender] -= trx->getValue();
          all_addrs_w_changed_balance.insert(receiver);
          expected_balances[receiver] += trx->getValue();
        }
      }
      if (opts.expect_to_fail) {
        EXPECT_EQ(r.status_code, 0);
      } else if (!opts.dont_assume_all_trx_success) {
        EXPECT_EQ(r.status_code, 1);
      }

      if (!opts.dont_assume_no_logs) {
        EXPECT_EQ(r.logs.size(), 0);
        EXPECT_EQ(r.bloom(), LogBloom());
      }
      expected_block_log_bloom |= r.bloom();
      auto trx_loc = *SUT->transactionLocation(trx->getHash());
      EXPECT_EQ(trx_loc.period, blk_h.number);
      EXPECT_EQ(trx_loc.position, i);
    }
    EXPECT_EQ(blk_h.gas_used, cumulative_gas_used_actual);
    if (!receipts.empty()) {
      EXPECT_EQ(receipts.back().cumulative_gas_used, cumulative_gas_used_actual);
    }
    EXPECT_EQ(blk_h.log_bloom, expected_block_log_bloom);
    if (assume_only_toplevel_transfers) {
      for (const auto& addr : all_addrs_w_changed_balance) {
        EXPECT_EQ(SUT->getAccount(addr)->balance, expected_balances[addr]);
      }
    }
    return result;
  }

  void fillConfigForGenesisTests(const addr_t& init_address) {
    cfg.genesis.state.initial_balances = {};
    cfg.genesis.state.initial_balances[init_address] = 1000000000 * kOneTara;
    cfg.genesis.state.dpos.eligibility_balance_threshold = 100000 * kOneTara;
    cfg.genesis.state.dpos.vote_eligibility_balance_step = 10000 * kOneTara;
    cfg.genesis.state.dpos.validator_maximum_stake = 10000000 * kOneTara;
    cfg.genesis.state.dpos.minimum_deposit = 1000 * kOneTara;
    cfg.genesis.state.dpos.eligibility_balance_threshold = 1000 * kOneTara;
    cfg.genesis.state.dpos.yield_percentage = 10;
    cfg.genesis.state.dpos.blocks_per_year = 1000;
  }

  template <class T, class U>
  static h256 trieRootOver(uint _itemCount, const T& _getKey, const U& _getValue) {
    dev::BytesMap m;
    for (uint i = 0; i < _itemCount; ++i) {
      m[_getKey(i)] = _getValue(i);
    }
    return hash256(m);
  }
};

TEST_F(FinalChainTest, rustModePruneDoesNotRunLegacyBatchPath) {
#ifdef RUSTAXA_ENABLE_FINAL_CHAIN
  init();
  EXPECT_THROW(SUT->prune(0), DbException);
#else
  GTEST_SKIP() << "FinalChain shim is disabled";
#endif
}

TEST_F(FinalChainTest, initial_balances) {
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[addr_t::random()] = taraxa::uint256_t("0x16345785D8A0000");    // 1
  cfg.genesis.state.initial_balances[addr_t::random()] = taraxa::uint256_t("0x56BC75E2D63100000");  // 1k
  cfg.genesis.state.initial_balances[addr_t::random()] =
      taraxa::uint256_t("0x204FCE5E3E25026110000000");  //  10 Billion
  init();
}

TEST_F(FinalChainTest, contract) {
  auto sender_keys = dev::KeyPair::create();
  const auto& addr = sender_keys.address();
  const auto& sk = sender_keys.secret();
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[addr] = taraxa::uint256_t("0x204FCE5E3E25026110000000");  //  10 Billion
  init();
  auto nonce = 0;
  auto trx =
      std::make_shared<Transaction>(nonce++, 0, 1000000000, 1000000, dev::fromHex(samples::greeter_contract_code), sk);
  auto result = advance({trx});
  auto contract_addr = result->trx_receipts[0].new_contract_address;
  std::cout << "contract_addr " << contract_addr->toString() << std::endl;
  EXPECT_EQ(contract_addr, dev::right160(dev::sha3(dev::rlpList(addr, 0))));
  auto greet = [&] {
    auto ret = SUT->call({
        addr,
        1000000000,
        contract_addr,
        0,
        0,
        1000000,
        // greet()
        dev::fromHex("0xcfae3217"),
    });
    return dev::toHexPrefixed(ret.code_retval);
  };
  ASSERT_EQ(greet(),
            // "Hello"
            "0x0000000000000000000000000000000000000000000000000000000000000020"
            "000000000000000000000000000000000000000000000000000000000000000548"
            "656c6c6f000000000000000000000000000000000000000000000000000000");
  {
    advance({
        std::make_shared<Transaction>(nonce++, 11, 1000000000, 1000000,
                                      // setGreeting("Hola")
                                      dev::fromHex("0xa4136862000000000000000000000000000000000000000000000000"
                                                   "00000000000000200000000000000000000000000000000000000000000"
                                                   "000000000000000000004486f6c61000000000000000000000000000000"
                                                   "00000000000000000000000000"),
                                      sk, contract_addr),
    });
  }
  ASSERT_EQ(greet(),
            // "Hola"
            "0x000000000000000000000000000000000000000000000000000000000000002000"
            "00000000000000000000000000000000000000000000000000000000000004486f"
            "6c6100000000000000000000000000000000000000000000000000000000");
}

TEST_F(FinalChainTest, coin_transfers) {
  constexpr size_t NUM_ACCS = 5;
  cfg.genesis.state.initial_balances = {};
  std::vector<dev::KeyPair> keys;
  keys.reserve(NUM_ACCS);
  for (size_t i = 0; i < NUM_ACCS; ++i) {
    const auto& k = keys.emplace_back(dev::KeyPair::create());
    cfg.genesis.state.initial_balances[k.address()] =
        taraxa::uint256_t("0x204FCE5E3E25026110000000") /* 10 Billion */ / NUM_ACCS;
  }

  init();
  advance({});

  constexpr auto TRX_GAS = 100000;
  advance({
      std::make_shared<Transaction>(1, 13, 1000000000, TRX_GAS, dev::bytes(), keys[0].secret(), keys[1].address()),
      std::make_shared<Transaction>(1, 11300, 1000000000, TRX_GAS, dev::bytes(), keys[1].secret(), keys[1].address()),
      std::make_shared<Transaction>(1, 1040, 1000000000, TRX_GAS, dev::bytes(), keys[2].secret(), keys[1].address()),
  });
  advance({});
  advance({
      std::make_shared<Transaction>(1, 0, 1000000000, TRX_GAS, dev::bytes(), keys[3].secret(), keys[1].address()),
      std::make_shared<Transaction>(1, 131, 1000000000, TRX_GAS, dev::bytes(), keys[4].secret(), keys[1].address()),
  });
  advance({
      std::make_shared<Transaction>(2, 100441, 1000000000, TRX_GAS, dev::bytes(), keys[0].secret(), keys[1].address()),
      std::make_shared<Transaction>(2, 2300, 1000000000, TRX_GAS, dev::bytes(), keys[1].secret(), keys[1].address()),
      std::make_shared<Transaction>(2, 130, 1000000000, TRX_GAS, dev::bytes(), keys[2].secret(), keys[1].address()),
  });
  advance({});
  advance({
      std::make_shared<Transaction>(2, 100431, 1000000000, TRX_GAS, dev::bytes(), keys[3].secret(), keys[1].address()),
      std::make_shared<Transaction>(2, 13411, 1000000000, TRX_GAS, dev::bytes(), keys[4].secret(), keys[1].address()),
      std::make_shared<Transaction>(3, 130, 1000000000, TRX_GAS, dev::bytes(), keys[0].secret(), keys[1].address()),
      std::make_shared<Transaction>(3, 343434, 1000000000, TRX_GAS, dev::bytes(), keys[1].secret(), keys[1].address()),
      std::make_shared<Transaction>(3, 131313, 1000000000, TRX_GAS, dev::bytes(), keys[2].secret(), keys[1].address()),
      std::make_shared<Transaction>(3, 143430, 1000000000, TRX_GAS, dev::bytes(), keys[3].secret(), keys[1].address()),
      std::make_shared<Transaction>(3, 1313145, 1000000000, TRX_GAS, dev::bytes(), keys[4].secret(), keys[1].address()),
  });
}

TEST_F(FinalChainTest, initial_validators) {
  const dev::KeyPair key = dev::KeyPair::create();
  const std::vector<dev::KeyPair> validator_keys = {dev::KeyPair::create(), dev::KeyPair::create(),
                                                    dev::KeyPair::create()};
  fillConfigForGenesisTests(key.address());

  for (const auto& vk : validator_keys) {
    const auto vrf_pub_key = taraxa::vrf_wrapper::getVrfKeyPair().first;
    state_api::ValidatorInfo validator{vk.address(), key.address(), vrf_pub_key, 0, "", "", {}};
    validator.delegations.emplace(key.address(), cfg.genesis.state.dpos.validator_maximum_stake);
    cfg.genesis.state.dpos.initial_validators.emplace_back(validator);
  }

  init();
  const auto votes_per_address =
      cfg.genesis.state.dpos.validator_maximum_stake / cfg.genesis.state.dpos.vote_eligibility_balance_step;
  const auto total_votes = SUT->dposEligibleTotalVoteCount(SUT->lastBlockNumber());
  for (const auto& vk : validator_keys) {
    const auto address_votes = SUT->dposEligibleVoteCount(SUT->lastBlockNumber(), vk.address());
    EXPECT_EQ(votes_per_address, address_votes);
    EXPECT_EQ(validator_keys.size() * votes_per_address, total_votes);
  }
}

TEST_F(FinalChainTest, nonce_test) {
  auto sender_keys = dev::KeyPair::create();
  const auto& addr = sender_keys.address();
  const auto& sk = sender_keys.secret();
  const auto receiver_addr = dev::KeyPair::create().address();
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[addr] = taraxa::uint256_t("0x204FCE5E3E25026110000000");  //  10 Billion
  init();

  auto trx1 = std::make_shared<Transaction>(0, 100, 1000000000, 100000, dev::bytes(), sk, receiver_addr);
  auto trx2 = std::make_shared<Transaction>(1, 100, 1000000000, 100000, dev::bytes(), sk, receiver_addr);
  auto trx3 = std::make_shared<Transaction>(2, 100, 1000000000, 100000, dev::bytes(), sk, receiver_addr);
  auto trx4 = std::make_shared<Transaction>(3, 100, 1000000000, 100000, dev::bytes(), sk, receiver_addr);

  advance({trx1});
  advance({trx2});
  advance({trx3});
  advance({trx4});

  ASSERT_EQ(SUT->getAccount(addr)->nonce.convert_to<uint64_t>(), 4);

  // nonce_skipping is enabled, ok
  auto trx6 = std::make_shared<Transaction>(6, 100, 1000000000, 100000, dev::bytes(), sk, receiver_addr);
  advance({trx6});

  ASSERT_EQ(SUT->getAccount(addr)->nonce.convert_to<uint64_t>(), 7);

  // nonce is lower, fail
  auto trx5 = std::make_shared<Transaction>(5, 101, 1000000000, 100000, dev::bytes(), sk, receiver_addr);
  advance({trx5}, {false, false, true});
}

TEST_F(FinalChainTest, nonce_skipping) {
  auto sender_keys = dev::KeyPair::create();
  const auto& addr = sender_keys.address();
  const auto& sk = sender_keys.secret();
  const auto receiver_addr = dev::KeyPair::create().address();
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[addr] = taraxa::uint256_t("0x204FCE5E3E25026110000000");  //  10 Billion
  init();

  auto trx1 = std::make_shared<Transaction>(0, 100, 1000000000, 100000, dev::bytes(), sk, receiver_addr);
  auto trx2 = std::make_shared<Transaction>(1, 100, 1000000000, 100000, dev::bytes(), sk, receiver_addr);
  auto trx3 = std::make_shared<Transaction>(2, 100, 1000000000, 100000, dev::bytes(), sk, receiver_addr);
  auto trx4 = std::make_shared<Transaction>(3, 100, 1000000000, 100000, dev::bytes(), sk, receiver_addr);

  advance({trx1});
  ASSERT_EQ(SUT->getAccount(addr)->nonce.convert_to<uint64_t>(), 1);

  advance({trx3});
  ASSERT_EQ(SUT->getAccount(addr)->nonce.convert_to<uint64_t>(), 3);

  // fail transaction with the same nonce
  advance({trx3}, {false, false, true});

  // fail transaction with lower nonce
  advance({trx2}, {false, false, true});

  ASSERT_EQ(SUT->getAccount(addr)->nonce.convert_to<uint64_t>(), 3);

  advance({trx4});
  ASSERT_EQ(SUT->getAccount(addr)->nonce.convert_to<uint64_t>(), 4);
}

TEST_F(FinalChainTest, exec_trx_with_nonce_from_api) {
  auto sender_keys = dev::KeyPair::create();
  const auto& addr = sender_keys.address();
  const auto& sk = sender_keys.secret();
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[addr] = taraxa::uint256_t("0x204FCE5E3E25026110000000");  //  10 Billion
  init();

  // exec trx with nonce 5 to skip some
  auto nonce = 5;
  {
    auto trx =
        std::make_shared<Transaction>(nonce, 0, 1000000000, 1000000, dev::fromHex(samples::greeter_contract_code), sk);
    auto result = advance({trx});
  }
  // fail second trx with same nonce
  {
    auto trx =
        std::make_shared<Transaction>(nonce, 1, 1000000000, 1000000, dev::fromHex(samples::greeter_contract_code), sk);
    auto result = advance({trx}, {false, false, true});
  }
  auto account = SUT->getAccount(addr);
  ASSERT_EQ(account->nonce, nonce + 1);
  auto trx = std::make_shared<Transaction>(account->nonce, 1, 1000000000, 1000000,
                                           dev::fromHex(samples::greeter_contract_code), sk);
  auto result = advance({trx});
}

TEST_F(FinalChainTest, new_contract_address) {
  auto new_contract_address = [](u256 nonce, const addr_t& sender) {
    return dev::right160(dev::sha3(dev::rlpList(sender, nonce)));
  };
  {
    const auto& sender = addr_t("0xbc3f916f3384eb088b2c662f59aca594a5b25b02");
    // https://rinkeby.etherscan.io/tx/0x2c4d922f7031584ade06b04aa661c6d045482450c36c1c844848adafca29c026
    // from: 0xbc3f916f3384eb088b2c662f59aca594a5b25b02 nonce:58 created: 0x313312e14cbdad86d616debd37e0ecf0b3dfef03
    // https://rinkeby.etherscan.io/tx/0x0923ba99c0839b9ef761e1e61b7b7f20eb9d3fd48a955ae34e591c9e27e0dcce
    // from: 0xbc3f916f3384eb088b2c662f59aca594a5b25b02 nonce:59 created: 0x22f95efe25ff7dce8ed5066acff5572f9f1683e8
    // https://rinkeby.etherscan.io/tx/0x555b5fa200f768da0df1c7141321b76251c45d63aeba8d2c840fa046128d92a6
    // from: 0xbc3f916f3384eb088b2c662f59aca594a5b25b02 nonce:60 created: 0x2109b75cca2094f5df48a7a2f8a4514b521038bc
    std::map<uint8_t, addr_t> nonce_and_address = {
        {58, addr_t("0x313312e14cbdad86d616debd37e0ecf0b3dfef03")},
        {59, addr_t("0x22f95efe25ff7dce8ed5066acff5572f9f1683e8")},
        {60, addr_t("0x2109b75cca2094f5df48a7a2f8a4514b521038bc")},
    };

    for (const auto& p : nonce_and_address) {
      ASSERT_EQ(new_contract_address(p.first, sender), p.second);
    }
  }

  auto sender_keys = dev::KeyPair::create();
  const auto& addr = sender_keys.address();
  const auto& sk = sender_keys.secret();
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[addr] = taraxa::uint256_t("0x204FCE5E3E25026110000000");  //  10 Billion
  init();

  auto nonce = 0;
  {
    auto trx =
        std::make_shared<Transaction>(nonce, 0, 1000000000, 1000000, dev::fromHex(samples::greeter_contract_code), sk);
    auto result = advance({trx});
    auto contract_addr = result->trx_receipts[0].new_contract_address;
    ASSERT_EQ(contract_addr, new_contract_address(trx->getNonce(), addr));
  }

  // skip few transactions, but new contract address should be still correct
  {
    nonce = 5;
    auto trx =
        std::make_shared<Transaction>(nonce, 0, 1000000000, 1000000, dev::fromHex(samples::greeter_contract_code), sk);
    auto result = advance({trx});
    auto contract_addr = result->trx_receipts[0].new_contract_address;
    ASSERT_EQ(contract_addr, new_contract_address(trx->getNonce(), addr));
  }
}

TEST_F(FinalChainTest, failed_transaction_fee) {
  auto sender_keys = dev::KeyPair::create();
  const auto gas = 30000;
  const uint256_t gas_price = 1000000000;

  const auto& receiver = dev::KeyPair::create().address();
  const auto& addr = sender_keys.address();
  const auto& sk = sender_keys.secret();
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[addr] = gas_price * gas * 4;  //  10 Billion
  init();
  auto trx1 = std::make_shared<Transaction>(1, 100, gas_price, gas, dev::bytes(), sk, receiver);
  auto trx2 = std::make_shared<Transaction>(2, 100, gas_price, gas, dev::bytes(), sk, receiver);
  auto trx3 = std::make_shared<Transaction>(3, 100, gas_price, gas, dev::bytes(), sk, receiver);
  auto trx2_1 = std::make_shared<Transaction>(2, 101, gas_price, gas, dev::bytes(), sk, receiver);

  advance({trx1});
  auto blk = SUT->blockHeader(expected_blk_num);
  auto proposer_balance = SUT->getBalance(blk->author);
  EXPECT_EQ(proposer_balance.first, trx1->getGasPrice() * 21000);
  advance({trx2});
  advance({trx3});

  {
    // low nonce trx should fail and consume all gas
    auto balance_before = SUT->getAccount(addr)->balance;
    advance({trx2_1}, {false, false, true});
    auto loc = SUT->transactionLocation(trx2_1->getHash());
    EXPECT_TRUE(loc.has_value());
    auto receipt = SUT->transactionReceipt(loc->period, loc->position);
    EXPECT_EQ(receipt->gas_used, gas);
    EXPECT_EQ(balance_before - SUT->getAccount(addr)->balance, receipt->gas_used * trx2_1->getGasPrice());
  }
  {
    // transaction gas is bigger then current account balance. Use closest int as gas used and decrease sender balance
    // by gas_used * gas_price

    ASSERT_GE(gas * gas_price, SUT->getAccount(addr)->balance);
    auto balance_before = SUT->getAccount(addr)->balance;
    auto trx4 = std::make_shared<Transaction>(4, 100, gas_price, gas, dev::bytes(), sk, receiver);
    advance({trx4}, {false, false, true});
    auto loc = SUT->transactionLocation(trx4->getHash());
    EXPECT_TRUE(loc.has_value());
    auto receipt = SUT->transactionReceipt(loc->period, loc->position);
    EXPECT_GT(balance_before % gas_price, 0);
    EXPECT_EQ(receipt->gas_used, balance_before / gas_price);
    EXPECT_EQ(SUT->getAccount(addr)->balance, balance_before % gas_price);
  }
}

TEST_F(FinalChainTest, revert_reason) {
  // contract TestRevert {
  //   function test(bool arg) public pure {
  //       require(arg, "arg required");
  //   }
  // }
  const auto test_contract_code =
      "608060405234801561001057600080fd5b506101ac806100206000396000f3fe608060405234801561001057600080fd5b50600436106100"
      "2b5760003560e01c806336091dff14610030575b600080fd5b61004a600480360381019061004591906100cc565b61004c565b005b806100"
      "8c576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161008390610156565b60405180"
      "910390fd5b50565b600080fd5b60008115159050919050565b6100a981610094565b81146100b457600080fd5b50565b6000813590506100"
      "c6816100a0565b92915050565b6000602082840312156100e2576100e161008f565b5b60006100f0848285016100b7565b91505092915050"
      "565b600082825260208201905092915050565b7f617267207265717569726564000000000000000000000000000000000000000060008201"
      "5250565b6000610140600c836100f9565b915061014b8261010a565b602082019050919050565b6000602082019050818103600083015261"
      "016f81610133565b905091905056fea2646970667358221220846c5a92aab30dade0d92661a25b1fd6ba9a914fd114f2f264c2003b5abdda"
      "db64736f6c63430008120033";
  auto sender_keys = dev::KeyPair::create();
  const auto& from = sender_keys.address();
  const auto& sk = sender_keys.secret();
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[from] = u256("10000000000000000000000");
  init();

  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.chain_id = cfg.genesis.chain_id;
  eth_rpc_params.gas_limit = cfg.genesis.dag.gas_limit;
  eth_rpc_params.final_chain = SUT;
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  auto nonce = 0;
  auto trx1 =
      std::make_shared<Transaction>(nonce++, 0, 1000000000, TEST_TX_GAS_LIMIT, dev::fromHex(test_contract_code), sk);
  auto result = advance({trx1});
  auto test_contract_addr = result->trx_receipts[0].new_contract_address;
  EXPECT_EQ(test_contract_addr, dev::right160(dev::sha3(dev::rlpList(from, 0))));
  auto call_data = "0x36091dff0000000000000000000000000000000000000000000000000000000000000000";
  {
    Json::Value est(Json::objectValue);
    est["to"] = dev::toHex(*test_contract_addr);
    est["from"] = dev::toHex(from);
    est["data"] = call_data;
    EXPECT_THROW_WITH(dev::jsToInt(eth_json_rpc->eth_estimateGas(est, "")), std::exception,
                      "execution reverted: arg required");
    EXPECT_THROW_WITH(
        eth_json_rpc->eth_call(est, "latest"), std::exception,
        "Exception 3 : execution reverted: arg required, data: "
        "\"0x08c379a000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000"
        "00000000000000000000000000000c6172672072657175697265640000000000000000000000000000000000000000\"\n");

    auto gas = 100000;
    auto trx = std::make_shared<Transaction>(2, 0, 1000000000, gas, dev::fromHex(call_data), sk, test_contract_addr);
    auto result = advance({trx}, {0, 0, 1});
    auto receipt = result->trx_receipts.front();
    ASSERT_EQ(receipt.status_code, 0);  // failed
    ASSERT_GT(gas, receipt.gas_used);   // we aren't spending all gas in such cases
  }
}

TEST_F(FinalChainTest, incorrect_estimation_regress) {
  // contract Receiver {
  //     uint256 public receivedETH;
  //     receive() external payable {
  //         receivedETH += msg.value;
  //     }
  // }
  const auto receiver_contract_code =
      "608060405234801561001057600080fd5b5061012d806100206000396000f3fe608060405260043610601f5760003560e01c8063820bec9d"
      "14603f57603a565b36603a57346000808282546032919060a4565b925050819055005b600080fd5b348015604a57600080fd5b5060516065"
      "565b604051605c919060de565b60405180910390f35b60005481565b6000819050919050565b7f4e487b7100000000000000000000000000"
      "000000000000000000000000000000600052601160045260246000fd5b600060ad82606b565b915060b683606b565b925082820190508082"
      "111560cb5760ca6075565b5b92915050565b60d881606b565b82525050565b600060208201905060f1600083018460d1565b9291505056fe"
      "a264697066735822122099ea1faf8b41cec96834060f2daaea3ae5c03561e110bdcf5a74ce041ddb497164736f6c63430008120033";

  // contract SendFunction {
  //     function send(address to) external payable {
  //         (bool success,) = to.call{value: msg.value}("");
  //         if (!success) {
  //             revert("Failed to send ETH");
  //         }
  //     }
  // }
  const auto sender_contract_code =
      "608060405234801561001057600080fd5b50610278806100206000396000f3fe60806040526004361061001e5760003560e01c80633e58c5"
      "8c14610023575b600080fd5b61003d60048036038101906100389190610152565b61003f565b005b60008173ffffffffffffffffffffffff"
      "ffffffffffffffff1634604051610065906101b0565b60006040518083038185875af1925050503d80600081146100a2576040519150601f"
      "19603f3d011682016040523d82523d6000602084013e6100a7565b606091505b50509050806100eb576040517f08c379a000000000000000"
      "00000000000000000000000000000000000000000081526004016100e290610222565b60405180910390fd5b5050565b600080fd5b600073"
      "ffffffffffffffffffffffffffffffffffffffff82169050919050565b600061011f826100f4565b9050919050565b61012f81610114565b"
      "811461013a57600080fd5b50565b60008135905061014c81610126565b92915050565b600060208284031215610168576101676100ef565b"
      "5b60006101768482850161013d565b91505092915050565b600081905092915050565b50565b600061019a60008361017f565b91506101a5"
      "8261018a565b600082019050919050565b60006101bb8261018d565b9150819050919050565b600082825260208201905092915050565b7f"
      "4661696c656420746f2073656e64204554480000000000000000000000000000600082015250565b600061020c6012836101c5565b915061"
      "0217826101d6565b602082019050919050565b6000602082019050818103600083015261023b816101ff565b905091905056fea264697066"
      "73582212205fd48a05d31cae1309b1a3bb8fe678c4bfee4cd28079acd90056ad228e18d82864736f6c63430008120033";

  auto sender_keys = dev::KeyPair::create();
  const auto& from = sender_keys.address();
  const auto& sk = sender_keys.secret();
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[from] = u256("10000000000000000000000");
  // disable balances check as we have internal transfer
  assume_only_toplevel_transfers = false;
  init();

  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.chain_id = cfg.genesis.chain_id;
  eth_rpc_params.gas_limit = cfg.genesis.dag.gas_limit;
  eth_rpc_params.final_chain = SUT;
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  auto nonce = 0;
  auto trx1 = std::make_shared<Transaction>(nonce++, 0, 1000000000, TEST_TX_GAS_LIMIT,
                                            dev::fromHex(receiver_contract_code), sk);
  auto trx2 =
      std::make_shared<Transaction>(nonce++, 0, 1000000000, TEST_TX_GAS_LIMIT, dev::fromHex(sender_contract_code), sk);
  auto result = advance({trx1, trx2});
  auto receiver_contract_addr = result->trx_receipts[0].new_contract_address;
  auto sender_contract_addr = result->trx_receipts[1].new_contract_address;
  EXPECT_EQ(receiver_contract_addr, dev::right160(dev::sha3(dev::rlpList(from, 0))));

  const auto call_data = "0x3e58c58c000000000000000000000000" + receiver_contract_addr->toString();
  const auto value = 10000;
  {
    Json::Value est(Json::objectValue);
    est["to"] = dev::toHex(*sender_contract_addr);
    est["from"] = dev::toHex(from);
    est["value"] = value;
    est["data"] = call_data;
    auto estimate = dev::jsToInt(eth_json_rpc->eth_estimateGas(est, ""));
    est["gas"] = dev::toJS(estimate);
    eth_json_rpc->eth_call(est, "latest");
  }
}

TEST_F(FinalChainTest, get_logs_multiple_topics) {
  // contract Events {
  //     event Event1(uint256 indexed v1);
  //     event Event2(uint256 indexed v1,uint256 indexed v2);
  //     event Event3(uint256 indexed v1,uint256 indexed v2,uint256 indexed v3);
  //     function method1(uint256 v1) public {
  //         emit Event1(v1);
  //     }
  //     function method2(uint256 v1, uint256 v2) public {
  //         emit Event2(v1, v2);
  //     }
  //     function method3(uint256 v1, uint256 v2, uint256 v3) public {
  //         emit Event3(v1, v2, v3);
  //     }
  // }
  const auto events_contract_code =
      "608060405234801561001057600080fd5b50610261806100206000396000f3fe608060405234801561001057600080fd5b50600436106100"
      "415760003560e01c8063110d99ed14610046578063d6f7f2a114610062578063ffcd960e1461007e575b600080fd5b610060600480360381"
      "019061005b919061016b565b61009a565b005b61007c60048036038101906100779190610198565b6100ca565b005b610098600480360381"
      "019061009391906101d8565b6100fc565b005b807f04474795f5b996ff80cb47c148d4c5ccdbe09ef27551820caa9c2f8ed149cce3604051"
      "60405180910390a250565b80827f6a822560072e19c1981d3d3bb11e5954a77efa0caf306eb08d053f37de0040ba60405160405180910390"
      "a35050565b8082847fac279a174af532aabe2bdfe61037bff7cfa74374d4d24034e97609940e4e2ac960405160405180910390a450505056"
      "5b600080fd5b6000819050919050565b61014881610135565b811461015357600080fd5b50565b6000813590506101658161013f565b9291"
      "5050565b60006020828403121561018157610180610130565b5b600061018f84828501610156565b91505092915050565b60008060408385"
      "0312156101af576101ae610130565b5b60006101bd85828601610156565b92505060206101ce85828601610156565b915050925092905056"
      "5b6000806000606084860312156101f1576101f0610130565b5b60006101ff86828701610156565b93505060206102108682870161015656"
      "5b925050604061022186828701610156565b915050925092509256fea264697066735822122005a8bf7a7bc842378d30f7446847533e0b35"
      "074e5453f29fe8762c0eb4d6f4ba64736f6c63430008120033";

  auto sender_keys = dev::KeyPair::create();
  const auto& from = sender_keys.address();
  const auto& sk = sender_keys.secret();
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[from] = u256("10000000000000000000000");
  init();

  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.chain_id = cfg.genesis.chain_id;
  eth_rpc_params.gas_limit = cfg.genesis.dag.gas_limit;
  eth_rpc_params.final_chain = SUT;
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  auto nonce = 0;

  auto trx1 =
      std::make_shared<Transaction>(nonce++, 0, 1000000000, TEST_TX_GAS_LIMIT, dev::fromHex(events_contract_code), sk);
  auto result = advance({trx1});
  auto contract_addr = result->trx_receipts[0].new_contract_address;

  auto to_call_param = [](uint64_t v) -> std::string {
    auto str = std::to_string(v);
    return std::string(64 - str.size(), '0') + str;
  };
  auto make_call_trx = [&](const std::string& method, const std::vector<uint64_t>& params) {
    auto params_str = std::accumulate(params.begin(), params.end(), std::string(),
                                      [&](const std::string& r, uint64_t p) { return r + to_call_param(p); });
    return std::make_shared<Transaction>(nonce++, 0, 1000000000, TEST_TX_GAS_LIMIT, dev::fromHex(method + params_str),
                                         sk, contract_addr);
  };
  auto method1 = "0x110d99ed";
  auto method2 = "0xd6f7f2a1";
  auto method3 = "0xffcd960e";
  auto topic1 = "0x04474795f5b996ff80cb47c148d4c5ccdbe09ef27551820caa9c2f8ed149cce3";
  auto topic2 = "0x6a822560072e19c1981d3d3bb11e5954a77efa0caf306eb08d053f37de0040ba";

  auto from_block = expected_blk_num;
  {
    auto trx = make_call_trx(method1, {1});
    advance({trx}, {true});
  }
  {
    auto trx = make_call_trx(method1, {2});
    advance({trx}, {true});
  }
  {
    auto trx = make_call_trx(method2, {1, 2});
    advance({trx}, {true});
  }
  {
    auto trx = make_call_trx(method3, {1, 2, 3});
    advance({trx}, {true});
  }
  {
    Json::Value topics{Json::arrayValue};
    topics.append(topic1);
    topics.append(topic2);
    Json::Value logs_obj(Json::objectValue);
    logs_obj["fromBlock"] = dev::toJS(from_block);
    logs_obj["address"] = contract_addr->toString();
    logs_obj["topics"] = Json::Value(Json::arrayValue);
    logs_obj["topics"].append(topics);
    auto res = eth_json_rpc->eth_getLogs(logs_obj);
    ASSERT_EQ(res.size(), 3);
  }
}

TEST_F(FinalChainTest, topics_size_limit) {
  init();

  net::rpc::eth::EthParams eth_rpc_params;
  eth_rpc_params.chain_id = cfg.genesis.chain_id;
  eth_rpc_params.gas_limit = cfg.genesis.dag.gas_limit;
  eth_rpc_params.final_chain = SUT;
  auto eth_json_rpc = net::rpc::eth::NewEth(std::move(eth_rpc_params));

  Json::Value logs_obj(Json::objectValue);
  logs_obj["topics"] = Json::Value(Json::arrayValue);
  logs_obj["topics"].append("1");
  logs_obj["topics"].append("2");
  logs_obj["topics"].append("3");
  logs_obj["topics"].append("4");
  eth_json_rpc->eth_getLogs(logs_obj);
  logs_obj["topics"].append("5");
  logs_obj["topics"].append("6");
  EXPECT_THROW(eth_json_rpc->eth_getLogs(logs_obj), jsonrpc::JsonRpcException);
}

TEST_F(FinalChainTest, fee_rewards_distribution) {
  auto sender_keys = dev::KeyPair::create();
  auto gas = 30000;

  const auto& receiver = dev::KeyPair::create().address();
  const auto& addr = sender_keys.address();
  const auto& sk = sender_keys.secret();
  cfg.genesis.state.initial_balances = {};
  cfg.genesis.state.initial_balances[addr] = taraxa::uint256_t("0x204FCE5E3E25026110000000");  //  10 Billion
  cfg.genesis.state.hardforks.magnolia_hf.block_num = 2;
  create_validators();
  init();
  const auto gas_price = 1000000000;
  {
    auto trx = std::make_shared<Transaction>(1, 100, gas_price, gas, dev::bytes(), sk, receiver);

    auto res = advance({trx});
    auto gas_used = res->trx_receipts.front().gas_used;
    EXPECT_EQ(SUT->getBalance(pbft_proposer_keys.address()).first, gas_used * gas_price);
  }
  {
    auto trx = std::make_shared<Transaction>(2, 100, gas_price, gas, dev::bytes(), sk, receiver);

    auto res = advance({trx});
    EXPECT_EQ(2, expected_blk_num);
    EXPECT_EQ(res->trx_receipts.size(), 1);
    auto gas_used = res->trx_receipts.front().gas_used;
    auto dags = db->getFinalizedDagBlockByPeriod(expected_blk_num);
    EXPECT_EQ(dags.size(), 1);
    EXPECT_EQ(SUT->getBalance(dag_proposer_keys.address()).first, 0);

    auto get_commission_rewards = [&](addr_t a) {
      const addr_t dpos_contract("0x00000000000000000000000000000000000000FE");
      auto ret = SUT->call({
          addr,
          gas_price,
          dpos_contract,
          0,
          0,
          1000000,
          // getValidator()
          dev::fromHex("0x1904bb2e000000000000000000000000" + a.toString()),
      });
      EXPECT_GE(ret.code_retval.size(), 96);
      // for some reason parsing u256 from bytes is failing check after
      auto hex_commission = "0x" + dev::toHex(bytes(ret.code_retval.begin() + 64, ret.code_retval.begin() + 96));
      return u256(hex_commission);
    };
    EXPECT_EQ(get_commission_rewards(dag_proposer_keys.address()), u256(gas_used * gas_price));
  }
}

std::shared_ptr<Transaction> makeDoubleVotingProofTx(const std::shared_ptr<PbftVote>& vote_a,
                                                     const std::shared_ptr<PbftVote>& vote_b, uint64_t nonce,
                                                     const dev::KeyPair& keys) {
  const auto kSlashingContractAddress = addr_t("0x00000000000000000000000000000000000000EE");
  // Create votes combination hash
  dev::RLPStream hash_rlp(2);
  if (vote_a->getHash() < vote_b->getHash()) {
    hash_rlp << vote_a->getHash();
    hash_rlp << vote_b->getHash();
  } else {
    hash_rlp << vote_b->getHash();
    hash_rlp << vote_a->getHash();
  }
  const auto hash_bytes = hash_rlp.invalidate();
  // const auto hash = dev::sha3(hash_bytes);

  auto input =
      util::EncodingSolidity::packFunctionCall("commitDoubleVotingProof(bytes,bytes)", vote_a->rlp(), vote_b->rlp());
  return std::make_shared<Transaction>(nonce, 0, 1000000000, 100000, std::move(input), keys.secret(),
                                       kSlashingContractAddress);
}

const addr_t kDposContractAddress = addr_t("0x00000000000000000000000000000000000000FE");

u256 get_validator_stake(const std::vector<state_api::ValidatorStake>& stakes, const addr_t& validator) {
  for (const auto& item : stakes) {
    if (item.addr == validator) {
      return item.stake;
    }
  }
  return 0;
}

bytes u256_to_padded_bytes32(const u256& value) {
  const auto value_bytes = dev::toBigEndian(value);
  bytes encoded(32, 0);
  std::copy(value_bytes.begin(), value_bytes.end(), encoded.begin() + (32 - value_bytes.size()));
  return encoded;
}

std::shared_ptr<Transaction> make_redelegate_tx(const FullNodeConfig& sender_cfg, const dev::KeyPair& sender,
                                                uint64_t nonce, const addr_t& from_validator,
                                                const addr_t& to_validator, const u256& amount, const u256& gas_price,
                                                const uint64_t gas_limit = TEST_TX_GAS_LIMIT) {
  return std::make_shared<Transaction>(nonce, 0, gas_price, gas_limit,
                                       util::EncodingSolidity::packFunctionCall("reDelegate(address,address,uint256)",
                                                                                from_validator, to_validator, amount),
                                       sender.secret(), kDposContractAddress, sender_cfg.genesis.chain_id);
}

TEST_F(FinalChainTest, remove_jailed_validator_votes_from_total) {
  const dev::KeyPair key = dev::KeyPair::create();
  const std::vector<dev::KeyPair> validator_keys = {dev::KeyPair::create(), dev::KeyPair::create(),
                                                    dev::KeyPair::create()};
  fillConfigForGenesisTests(key.address());
  cfg.genesis.state.hardforks.magnolia_hf.block_num = 1;
  cfg.genesis.state.hardforks.magnolia_hf.jail_time = 50;

  for (const auto& vk : validator_keys) {
    const auto vrf_pub_key = taraxa::vrf_wrapper::getVrfKeyPair().first;
    state_api::ValidatorInfo validator{vk.address(), key.address(), vrf_pub_key, 0, "", "", {}};
    validator.delegations.emplace(key.address(), cfg.genesis.state.dpos.validator_maximum_stake);
    cfg.genesis.state.dpos.initial_validators.emplace_back(validator);
  }

  init();
  const auto votes_per_address =
      cfg.genesis.state.dpos.validator_maximum_stake / cfg.genesis.state.dpos.vote_eligibility_balance_step;
  const auto total_votes_before = SUT->dposEligibleTotalVoteCount(SUT->lastBlockNumber());
  EXPECT_EQ(validator_keys.size() * votes_per_address, total_votes_before);
  for (const auto& vk : validator_keys) {
    const auto address_votes = SUT->dposEligibleVoteCount(SUT->lastBlockNumber(), vk.address());
    EXPECT_EQ(votes_per_address, address_votes);
  }

  advance({});
  // submit double votes for one validator
  const auto [vrf_key, vrf_sk] = taraxa::vrf_wrapper::getVrfKeyPair();
  VrfPbftSortition vrf_sortition(vrf_sk, {PbftVoteTypes::propose_vote, 1, 1, 1});
  auto vote_a = std::make_shared<PbftVote>(validator_keys[0].secret(), vrf_sortition, blk_hash_t(1));
  vote_a->calculateWeight(1, 1, 1);
  auto vote_b = std::make_shared<PbftVote>(validator_keys[0].secret(), vrf_sortition, blk_hash_t(2));
  vote_b->calculateWeight(1, 1, 1);

  auto trx = makeDoubleVotingProofTx(vote_a, vote_b, 1, key);
  auto res = advance({trx}, {true});
  for (size_t idx = 0; idx < cfg.genesis.state.dpos.delegation_delay; idx++) {
    advance({});
  }

  const auto total_votes = SUT->dposEligibleTotalVoteCount(SUT->lastBlockNumber());
  EXPECT_EQ(total_votes_before - votes_per_address, total_votes);
}

TEST_F(FinalChainTest, native_slashing_value_custody_commits_only_for_successful_proof) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kGasLimit = 200'000;
  constexpr uint64_t kFirstValue = 777;
  constexpr uint64_t kFailedValue = 888;
  constexpr uint64_t kJailTime = 50;
  constexpr uint64_t kSubmitterInitialBalance = 10'000'000;
  const addr_t kSlashingContract("0x00000000000000000000000000000000000000EE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair submitter{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kInitialStake + 1'000;
  cfg.genesis.state.initial_balances[submitter.address()] = kSubmitterInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = 1'000;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = 1'000;
  cfg.genesis.state.dpos.validator_maximum_stake = 30'000;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;
  cfg.genesis.state.hardforks.magnolia_hf.block_num = 1;
  cfg.genesis.state.hardforks.magnolia_hf.jail_time = kJailTime;
  cfg.genesis.state.hardforks.cacti_hf.block_num = 1;
  cfg.genesis.state.hardforks.cacti_hf.jail_time = kJailTime;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  init();
  advance({});

  const auto [vote_vrf_key, vote_vrf_secret] = vrf_wrapper::getVrfKeyPair();
  (void)vote_vrf_key;
  VrfPbftSortition vote_sortition(vote_vrf_secret, {PbftVoteTypes::propose_vote, 1, 1, 1});
  auto vote_a = std::make_shared<PbftVote>(validator.secret(), vote_sortition, blk_hash_t(1));
  vote_a->calculateWeight(1, 1, 1);
  auto vote_b = std::make_shared<PbftVote>(validator.secret(), vote_sortition, blk_hash_t(2));
  vote_b->calculateWeight(1, 1, 1);
  const auto input =
      util::EncodingSolidity::packFunctionCall("commitDoubleVotingProof(bytes,bytes)", vote_a->rlp(), vote_b->rlp());

  const auto successful_tx = std::make_shared<Transaction>(0, kFirstValue, kGasPrice, kGasLimit, input,
                                                           submitter.secret(), kSlashingContract, cfg.genesis.chain_id);
  const auto duplicate_tx = std::make_shared<Transaction>(1, kFailedValue, kGasPrice, kGasLimit, input,
                                                          submitter.secret(), kSlashingContract, cfg.genesis.chain_id);
  const auto result =
      advance({successful_tx, duplicate_tx}, {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true});
  ASSERT_EQ(result->trx_receipts.size(), 2);

  const auto proof_gas = IntrinsicGas(input, false) + 20'000;
  const auto expected_jail_block = uint64_t{2} + kJailTime;
  bytes behaviour(32, 0);
  behaviour.back() = 1;
  const LogEntry jailed_log{
      kSlashingContract,
      {dev::sha3(dev::asBytes("Jailed(address,uint64,uint64,uint8)")), h256(validator.address(), h256::AlignRight),
       h256(u256(2)), h256(u256(expected_jail_block))},
      behaviour};

  TransactionReceipt expected_success;
  expected_success.status_code = 1;
  expected_success.gas_used = proof_gas;
  expected_success.cumulative_gas_used = proof_gas;
  expected_success.logs = {jailed_log};
  TransactionReceipt expected_duplicate;
  expected_duplicate.status_code = 0;
  expected_duplicate.gas_used = proof_gas;
  expected_duplicate.cumulative_gas_used = proof_gas * 2;

  EXPECT_EQ(util::rlp_enc(result->trx_receipts[0]), util::rlp_enc(expected_success));
  EXPECT_EQ(util::rlp_enc(result->trx_receipts[1]), util::rlp_enc(expected_duplicate));
  EXPECT_EQ(result->trx_receipts[0].bloom(), expected_success.bloom());
  EXPECT_EQ(result->trx_receipts[1].bloom(), LogBloom());
  EXPECT_EQ(result->final_chain_blk->gas_used, proof_gas * 2);
  EXPECT_EQ(result->final_chain_blk->log_bloom, expected_success.bloom());

  const auto assert_persisted_state = [&](const std::shared_ptr<FinalChain>& chain) {
    const auto sender_account = chain->getAccount(submitter.address());
    ASSERT_TRUE(sender_account);
    EXPECT_EQ(sender_account->nonce, 2);
    EXPECT_EQ(sender_account->balance, u256(kSubmitterInitialBalance - kFirstValue - proof_gas * kGasPrice * 2));
    const auto slashing_account = chain->getAccount(kSlashingContract);
    ASSERT_TRUE(slashing_account);
    EXPECT_EQ(slashing_account->nonce, 1);
    EXPECT_EQ(slashing_account->balance, u256(kFirstValue));

    EXPECT_EQ(chain->dposEligibleVoteCount(2, validator.address()), 0);

    const auto persisted_success = chain->transactionReceipt(2, 0);
    const auto persisted_duplicate = chain->transactionReceipt(2, 1);
    ASSERT_TRUE(persisted_success);
    ASSERT_TRUE(persisted_duplicate);
    EXPECT_EQ(util::rlp_enc(*persisted_success), util::rlp_enc(expected_success));
    EXPECT_EQ(util::rlp_enc(*persisted_duplicate), util::rlp_enc(expected_duplicate));
  };

  assert_persisted_state(SUT);
  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 2);
  assert_persisted_state(SUT);
}

TEST_F(FinalChainTest, native_slashing_reads_preserve_value_gas_and_nonce_semantics) {
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kGasLimit = 100'000;
  constexpr uint64_t kJailBlockValue = 111;
  constexpr uint64_t kJailedValidatorsValue = 222;
  constexpr uint64_t kMalformedValue = 333;
  constexpr uint64_t kTransferValue = 444;
  constexpr uint64_t kSenderInitialBalance = 10'000'000;
  const addr_t kSlashingContract("0x00000000000000000000000000000000000000EE");

  const dev::KeyPair sender{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};
  const dev::KeyPair queried_validator{dev::Secret("5555555555555555555555555555555555555555555555555555555555555555")};
  const dev::KeyPair receiver{dev::Secret("6666666666666666666666666666666666666666666666666666666666666666")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[sender.address()] = kSenderInitialBalance;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.hardforks.magnolia_hf.block_num = 1;

  init();
  advance({});

  auto jail_block_input =
      util::EncodingSolidity::packFunctionCall("getJailBlock(address)", queried_validator.address());
  auto jailed_validators_input = util::EncodingSolidity::packFunctionCall("getJailedValidators()");
  jailed_validators_input.insert(jailed_validators_input.end(), {0xaa, 0xbb});
  auto malformed_jail_block_input = jail_block_input;
  malformed_jail_block_input.resize(4);

  const auto jail_block_tx = std::make_shared<Transaction>(0, kJailBlockValue, kGasPrice, kGasLimit, jail_block_input,
                                                           sender.secret(), kSlashingContract, cfg.genesis.chain_id);
  const auto jailed_validators_tx =
      std::make_shared<Transaction>(1, kJailedValidatorsValue, kGasPrice, kGasLimit, jailed_validators_input,
                                    sender.secret(), kSlashingContract, cfg.genesis.chain_id);
  const auto malformed_tx =
      std::make_shared<Transaction>(2, kMalformedValue, kGasPrice, kGasLimit, malformed_jail_block_input,
                                    sender.secret(), kSlashingContract, cfg.genesis.chain_id);
  const auto continuation_tx = std::make_shared<Transaction>(3, kTransferValue, kGasPrice, kGasLimit, bytes{},
                                                             sender.secret(), receiver.address(), cfg.genesis.chain_id);

  const auto result = advance({jail_block_tx, jailed_validators_tx, malformed_tx, continuation_tx},
                              {.dont_assume_all_trx_success = true});
  ASSERT_EQ(result->trx_receipts.size(), 4);

  const std::array<uint64_t, 4> expected_gas = {
      IntrinsicGas(jail_block_input, false) + 5'000, IntrinsicGas(jailed_validators_input, false) + 5'000,
      IntrinsicGas(malformed_jail_block_input, false) + 5'000, IntrinsicGas(bytes{}, false)};
  const std::array<uint64_t, 4> expected_status = {1, 1, 0, 1};
  uint64_t cumulative_gas = 0;
  for (size_t idx = 0; idx < result->trx_receipts.size(); ++idx) {
    cumulative_gas += expected_gas[idx];
    TransactionReceipt expected;
    expected.status_code = expected_status[idx];
    expected.gas_used = expected_gas[idx];
    expected.cumulative_gas_used = cumulative_gas;
    EXPECT_EQ(util::rlp_enc(result->trx_receipts[idx]), util::rlp_enc(expected));
    EXPECT_EQ(result->trx_receipts[idx].bloom(), LogBloom());
  }
  EXPECT_EQ(result->final_chain_blk->gas_used, cumulative_gas);
  EXPECT_EQ(result->final_chain_blk->log_bloom, LogBloom());

  const auto assert_persisted_state = [&](const std::shared_ptr<FinalChain>& chain) {
    const auto sender_account = chain->getAccount(sender.address());
    ASSERT_TRUE(sender_account);
    EXPECT_EQ(sender_account->nonce, 4);
    EXPECT_EQ(sender_account->balance,
              u256(kSenderInitialBalance - (kJailBlockValue + kJailedValidatorsValue + kTransferValue) -
                   cumulative_gas * kGasPrice));
    const auto slashing_account = chain->getAccount(kSlashingContract);
    ASSERT_TRUE(slashing_account);
    EXPECT_EQ(slashing_account->nonce, 0);
    EXPECT_EQ(slashing_account->balance, u256(kJailBlockValue + kJailedValidatorsValue));
    const auto receiver_account = chain->getAccount(receiver.address());
    ASSERT_TRUE(receiver_account);
    EXPECT_EQ(receiver_account->nonce, 0);
    EXPECT_EQ(receiver_account->balance, u256(kTransferValue));

    for (size_t idx = 0; idx < result->trx_receipts.size(); ++idx) {
      const auto persisted = chain->transactionReceipt(2, idx);
      ASSERT_TRUE(persisted);
      EXPECT_EQ(util::rlp_enc(*persisted), util::rlp_enc(result->trx_receipts[idx]));
    }
  };

  assert_persisted_state(SUT);
  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 2);
  assert_persisted_state(SUT);
}

TEST_F(FinalChainTest, native_dpos_delegate_persists_receipt_and_state) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kDelegation = 1'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kExpectedGas = 62'680;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair delegator{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  ASSERT_NE(owner.address(), validator.address());
  ASSERT_NE(owner.address(), delegator.address());
  ASSERT_NE(validator.address(), delegator.address());

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_account = SUT->getAccount(kDposContract);
  ASSERT_TRUE(initial_dpos_account);
  EXPECT_EQ(initial_dpos_account->nonce, 1);
  EXPECT_EQ(initial_dpos_account->balance, u256(kInitialStake));

  const auto calldata = dev::fromHex("5c19a95c000000000000000000000000" + validator.address().toString());
  ASSERT_EQ(calldata.size(), 36);
  EXPECT_EQ(bytes(calldata.begin(), calldata.begin() + 4), dev::fromHex("5c19a95c"));
  EXPECT_EQ(bytes(calldata.begin() + 4, calldata.end()),
            dev::fromHex("000000000000000000000000" + validator.address().toString()));

  auto transaction = std::make_shared<Transaction>(0, kDelegation, kGasPrice, TEST_TX_GAS_LIMIT, calldata,
                                                   delegator.secret(), kDposContract, cfg.genesis.chain_id);
  const auto result = advance({transaction}, {.dont_assume_no_logs = true});
  ASSERT_EQ(result->trx_receipts.size(), 1);

  bytes delegated_amount(32, 0);
  delegated_amount[30] = 0x03;
  delegated_amount[31] = 0xe8;
  const LogEntry delegated_log{
      kDposContract,
      {dev::sha3(dev::asBytes("Delegated(address,address,uint256)")), h256(delegator.address(), h256::AlignRight),
       h256(validator.address(), h256::AlignRight)},
      delegated_amount};
  TransactionReceipt expected_receipt;
  expected_receipt.status_code = 1;
  expected_receipt.gas_used = kExpectedGas;
  expected_receipt.cumulative_gas_used = kExpectedGas;
  expected_receipt.logs = {delegated_log};

  auto assert_persisted_delegate = [&](const std::shared_ptr<FinalChain>& chain) {
    const auto receipt = chain->transactionReceipt(1, 0);
    ASSERT_TRUE(receipt);
    EXPECT_EQ(receipt->status_code, 1);
    EXPECT_EQ(receipt->gas_used, kExpectedGas);
    EXPECT_EQ(receipt->cumulative_gas_used, kExpectedGas);
    ASSERT_EQ(receipt->logs.size(), 1);
    EXPECT_EQ(receipt->logs[0].address, delegated_log.address);
    EXPECT_EQ(receipt->logs[0].topics, delegated_log.topics);
    EXPECT_EQ(receipt->logs[0].data, delegated_log.data);
    EXPECT_EQ(util::rlp_enc(*receipt), util::rlp_enc(expected_receipt));
    EXPECT_EQ(receipt->bloom(), expected_receipt.bloom());

    const auto header = chain->blockHeader(1);
    ASSERT_TRUE(header);
    EXPECT_EQ(header->gas_used, kExpectedGas);
    EXPECT_EQ(header->log_bloom, expected_receipt.bloom());

    const auto delegator_account = chain->getAccount(delegator.address());
    ASSERT_TRUE(delegator_account);
    EXPECT_EQ(delegator_account->nonce, 1);
    EXPECT_EQ(delegator_account->balance, u256(kDelegatorInitialBalance - kDelegation - kExpectedGas * kGasPrice));
    const auto dpos_account = chain->getAccount(kDposContract);
    ASSERT_TRUE(dpos_account);
    EXPECT_EQ(dpos_account->nonce, 1);
    EXPECT_EQ(dpos_account->balance, u256(kInitialStake + kDelegation));
    EXPECT_EQ(chain->dposTotalAmountDelegated(1), u256(kInitialStake + kDelegation));

    const auto stakes = chain->dposValidatorsTotalStakes(1);
    ASSERT_EQ(stakes.size(), 1);
    EXPECT_EQ(stakes[0].addr, validator.address());
    EXPECT_EQ(stakes[0].stake, u256(kInitialStake + kDelegation));
    EXPECT_EQ(chain->dposEligibleVoteCount(1, validator.address()), 11);
    EXPECT_EQ(chain->dposEligibleTotalVoteCount(1), 11);
  };

  EXPECT_EQ(util::rlp_enc(result->trx_receipts[0]), util::rlp_enc(expected_receipt));
  EXPECT_EQ(result->final_chain_blk->log_bloom, expected_receipt.bloom());
  assert_persisted_delegate(SUT);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 1);
  assert_persisted_delegate(SUT);
}

TEST_F(FinalChainTest, native_dpos_delegate_to_missing_validator_rolls_back_state) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kDelegation = 1'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kExpectedGas = 61'464;
  constexpr uint64_t kGasLimit = 100'000;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair delegator{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const auto missing_validator = addr_t("0x0000000000000000000000000000000000000001");
  ASSERT_NE(owner.address(), validator.address());
  ASSERT_NE(owner.address(), delegator.address());
  ASSERT_NE(validator.address(), delegator.address());
  ASSERT_NE(validator.address(), missing_validator);
  ASSERT_NE(delegator.address(), missing_validator);

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_account = SUT->getAccount(kDposContract);
  ASSERT_TRUE(initial_dpos_account);
  const auto initial_dpos_balance = initial_dpos_account->balance;
  EXPECT_EQ(initial_dpos_account->nonce, 1);
  EXPECT_EQ(initial_dpos_balance, u256(kInitialStake));

  const auto calldata = dev::fromHex("5c19a95c0000000000000000000000000000000000000000000000000000000000000001");
  ASSERT_EQ(calldata.size(), 36);
  EXPECT_EQ(bytes(calldata.begin(), calldata.begin() + 4), dev::fromHex("5c19a95c"));
  EXPECT_EQ(bytes(calldata.begin() + 4, calldata.end()),
            dev::fromHex("000000000000000000000000" + missing_validator.toString()));

  auto transaction = std::make_shared<Transaction>(0, kDelegation, kGasPrice, kGasLimit, calldata, delegator.secret(),
                                                   kDposContract, cfg.genesis.chain_id);
  const auto result = advance(
      {transaction}, {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true, .expect_to_fail = true});
  ASSERT_EQ(result->trx_receipts.size(), 1);

  TransactionReceipt expected_receipt;
  expected_receipt.status_code = 0;
  expected_receipt.gas_used = kExpectedGas;
  expected_receipt.cumulative_gas_used = kExpectedGas;

  auto assert_failed_delegate_persists = [&](const std::shared_ptr<FinalChain>& chain) {
    const auto receipt = chain->transactionReceipt(1, 0);
    ASSERT_TRUE(receipt);
    EXPECT_EQ(receipt->status_code, 0);
    EXPECT_EQ(receipt->gas_used, kExpectedGas);
    EXPECT_EQ(receipt->cumulative_gas_used, kExpectedGas);
    EXPECT_EQ(receipt->logs.size(), 0);
    EXPECT_EQ(receipt->bloom(), expected_receipt.bloom());
    EXPECT_EQ(util::rlp_enc(*receipt), util::rlp_enc(expected_receipt));

    const auto header = chain->blockHeader(1);
    ASSERT_TRUE(header);
    EXPECT_EQ(header->gas_used, kExpectedGas);
    EXPECT_EQ(header->log_bloom, expected_receipt.bloom());

    const auto delegator_account = chain->getAccount(delegator.address());
    ASSERT_TRUE(delegator_account);
    EXPECT_EQ(delegator_account->nonce, 1);
    EXPECT_EQ(delegator_account->balance, u256(kDelegatorInitialBalance - kExpectedGas * kGasPrice));

    const auto dpos_account = chain->getAccount(kDposContract);
    ASSERT_TRUE(dpos_account);
    EXPECT_EQ(dpos_account->nonce, 1);
    EXPECT_EQ(dpos_account->balance, initial_dpos_balance);

    EXPECT_EQ(chain->dposTotalAmountDelegated(1), u256(kInitialStake));

    const auto stakes = chain->dposValidatorsTotalStakes(1);
    ASSERT_EQ(stakes.size(), 1);
    EXPECT_EQ(stakes[0].addr, validator.address());
    EXPECT_EQ(stakes[0].stake, u256(kInitialStake));
    EXPECT_EQ(chain->dposEligibleVoteCount(1, validator.address()), 10);
    EXPECT_EQ(chain->dposEligibleTotalVoteCount(1), 10);
  };

  EXPECT_EQ(util::rlp_enc(result->trx_receipts[0]), util::rlp_enc(expected_receipt));
  EXPECT_EQ(result->final_chain_blk->log_bloom, expected_receipt.bloom());
  assert_failed_delegate_persists(SUT);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 1);
  assert_failed_delegate_persists(SUT);
}

TEST_F(FinalChainTest, native_dpos_redelegate_persists_receipt_and_state) {
  constexpr uint64_t kDestinationStake = 3'000;
  constexpr uint64_t kDelegation = 5'000;
  constexpr uint64_t kRedelegation = kDelegation;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kOwnerInitialBalance = 100'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair source_validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair destination_validator{
      dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const dev::KeyPair delegator{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo source_validator_info{
      source_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  source_validator_info.delegations.emplace(delegator.address(), kDelegation);
  state_api::ValidatorInfo destination_validator_info{
      destination_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  destination_validator_info.delegations.emplace(owner.address(), kDestinationStake);
  cfg.genesis.state.dpos.initial_validators = {source_validator_info, destination_validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_account = SUT->getAccount(kDposContract);
  ASSERT_TRUE(initial_dpos_account);
  ASSERT_EQ(initial_dpos_account->nonce, 1);
  const auto initial_dpos_balance = initial_dpos_account->balance;
  const auto initial_total = SUT->dposTotalAmountDelegated(0);
  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);
  const auto initial_destination_stake = get_validator_stake(initial_stakes, destination_validator.address());

  const auto redelegate_tx = make_redelegate_tx(cfg, delegator, 0, source_validator.address(),
                                                destination_validator.address(), u256(kRedelegation), kGasPrice);
  const auto result = advance({redelegate_tx}, {.dont_assume_no_logs = true});
  ASSERT_EQ(result->trx_receipts.size(), 1);

  const auto redelegated_amount = u256_to_padded_bytes32(u256(kRedelegation));
  const LogEntry redelegated_log{
      kDposContract,
      {dev::sha3(dev::asBytes("Redelegated(address,address,address,uint256)")),
       h256(delegator.address(), h256::AlignRight), h256(source_validator.address(), h256::AlignRight),
       h256(destination_validator.address(), h256::AlignRight)},
      redelegated_amount};
  TransactionReceipt expected_receipt;
  expected_receipt.status_code = 1;
  const auto expected_gas = IntrinsicGas(redelegate_tx->getData(), false) + 80'000;
  expected_receipt.gas_used = expected_gas;
  expected_receipt.cumulative_gas_used = expected_gas;
  expected_receipt.logs = {redelegated_log};

  auto assert_persisted_redelegate = [&](const std::shared_ptr<FinalChain>& chain) {
    const auto receipt = chain->transactionReceipt(1, 0);
    ASSERT_TRUE(receipt);
    EXPECT_EQ(receipt->status_code, 1);
    EXPECT_EQ(receipt->gas_used, expected_gas);
    EXPECT_EQ(receipt->cumulative_gas_used, expected_gas);
    ASSERT_EQ(receipt->logs.size(), 1);
    EXPECT_EQ(receipt->logs[0].address, redelegated_log.address);
    EXPECT_EQ(receipt->logs[0].topics, redelegated_log.topics);
    EXPECT_EQ(receipt->logs[0].data, redelegated_log.data);
    EXPECT_EQ(util::rlp_enc(*receipt), util::rlp_enc(expected_receipt));
    EXPECT_EQ(receipt->bloom(), expected_receipt.bloom());

    const auto header = chain->blockHeader(1);
    ASSERT_TRUE(header);
    EXPECT_EQ(header->gas_used, expected_gas);
    EXPECT_EQ(header->log_bloom, expected_receipt.bloom());

    const auto delegator_account = chain->getAccount(delegator.address());
    ASSERT_TRUE(delegator_account);
    EXPECT_EQ(delegator_account->nonce, 1);
    EXPECT_EQ(delegator_account->balance, u256(kDelegatorInitialBalance - kDelegation - expected_gas * kGasPrice));

    const auto dpos_account = chain->getAccount(kDposContract);
    ASSERT_TRUE(dpos_account);
    EXPECT_EQ(dpos_account->nonce, 1);
    EXPECT_EQ(dpos_account->balance, initial_dpos_balance);

    EXPECT_EQ(chain->dposTotalAmountDelegated(1), initial_total);
    const auto current_stakes = chain->dposValidatorsTotalStakes(1);
    ASSERT_EQ(current_stakes.size(), 1);
    EXPECT_EQ(get_validator_stake(current_stakes, source_validator.address()), 0);
    EXPECT_EQ(get_validator_stake(current_stakes, destination_validator.address()),
              initial_destination_stake + kRedelegation);
  };

  EXPECT_EQ(util::rlp_enc(result->trx_receipts[0]), util::rlp_enc(expected_receipt));
  EXPECT_EQ(result->final_chain_blk->log_bloom, expected_receipt.bloom());
  assert_persisted_redelegate(SUT);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 1);
  assert_persisted_redelegate(SUT);
}

TEST_F(FinalChainTest, native_dpos_redelegate_to_missing_validator_rolls_back_state) {
  constexpr uint64_t kStake = 10'000;
  constexpr uint64_t kDelegation = 5'000;
  constexpr uint64_t kRedelegation = 1'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kGasLimit = TEST_TX_GAS_LIMIT;
  constexpr uint64_t kOwnerInitialBalance = 100'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");
  const addr_t kMissingValidator("0x0000000000000000000000000000000000000000");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair source_validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair delegator{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo source_validator_info{
      source_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  source_validator_info.delegations.emplace(owner.address(), kStake);
  source_validator_info.delegations.emplace(delegator.address(), kDelegation);
  cfg.genesis.state.dpos.initial_validators = {source_validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_account = SUT->getAccount(kDposContract);
  ASSERT_TRUE(initial_dpos_account);
  const auto initial_dpos_balance = initial_dpos_account->balance;
  const auto initial_total = SUT->dposTotalAmountDelegated(0);
  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);

  auto redelegate_tx = std::make_shared<Transaction>(
      0, 0, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("reDelegate(address,address,uint256)", source_validator.address(),
                                               kMissingValidator, u256(kRedelegation)),
      delegator.secret(), kDposContract, cfg.genesis.chain_id);
  const auto result = advance(
      {redelegate_tx}, {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true, .expect_to_fail = true});
  ASSERT_EQ(result->trx_receipts.size(), 1);

  const auto expected_gas = IntrinsicGas(redelegate_tx->getData(), false) + 80'000;
  TransactionReceipt expected_receipt;
  expected_receipt.status_code = 0;
  expected_receipt.gas_used = expected_gas;
  expected_receipt.cumulative_gas_used = expected_gas;

  auto assert_failed_redelegate_persists = [&](const std::shared_ptr<FinalChain>& chain) {
    const auto receipt = chain->transactionReceipt(1, 0);
    ASSERT_TRUE(receipt);
    EXPECT_EQ(receipt->status_code, 0);
    EXPECT_EQ(receipt->gas_used, expected_gas);
    EXPECT_EQ(receipt->cumulative_gas_used, expected_gas);
    EXPECT_EQ(receipt->logs.size(), 0);
    EXPECT_EQ(receipt->bloom(), expected_receipt.bloom());
    EXPECT_EQ(util::rlp_enc(*receipt), util::rlp_enc(expected_receipt));

    const auto header = chain->blockHeader(1);
    ASSERT_TRUE(header);
    EXPECT_EQ(header->gas_used, expected_gas);
    EXPECT_EQ(header->log_bloom, expected_receipt.bloom());

    const auto delegator_account = chain->getAccount(delegator.address());
    ASSERT_TRUE(delegator_account);
    EXPECT_EQ(delegator_account->nonce, 1);
    EXPECT_EQ(delegator_account->balance, u256(kDelegatorInitialBalance - kDelegation - expected_gas * kGasPrice));

    const auto dpos_account = chain->getAccount(kDposContract);
    ASSERT_TRUE(dpos_account);
    EXPECT_EQ(dpos_account->nonce, 1);
    EXPECT_EQ(dpos_account->balance, initial_dpos_balance);
    EXPECT_EQ(chain->dposTotalAmountDelegated(1), initial_total);

    const auto current_stakes = chain->dposValidatorsTotalStakes(1);
    ASSERT_EQ(current_stakes.size(), initial_stakes.size());
    EXPECT_EQ(get_validator_stake(current_stakes, source_validator.address()),
              get_validator_stake(initial_stakes, source_validator.address()));
  };

  EXPECT_EQ(util::rlp_enc(result->trx_receipts[0]), util::rlp_enc(expected_receipt));
  EXPECT_EQ(result->final_chain_blk->log_bloom, expected_receipt.bloom());
  assert_failed_redelegate_persists(SUT);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 1);
  assert_failed_redelegate_persists(SUT);
}

TEST_F(FinalChainTest, native_dpos_redelegate_to_maxed_validator_rolls_back_state) {
  constexpr uint64_t kSourceStake = 10'000;
  constexpr uint64_t kDestinationStake = 30'000;
  constexpr uint64_t kDelegation = 5'000;
  constexpr uint64_t kRedelegation = 1'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kGasLimit = TEST_TX_GAS_LIMIT;
  constexpr uint64_t kOwnerInitialBalance = 100'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair source_validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair destination_validator{
      dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const dev::KeyPair delegator{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo source_validator_info{
      source_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  source_validator_info.delegations.emplace(owner.address(), kSourceStake);
  source_validator_info.delegations.emplace(delegator.address(), kDelegation);
  state_api::ValidatorInfo destination_validator_info{
      destination_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  destination_validator_info.delegations.emplace(owner.address(), kDestinationStake);
  cfg.genesis.state.dpos.initial_validators = {source_validator_info, destination_validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_account = SUT->getAccount(kDposContract);
  ASSERT_TRUE(initial_dpos_account);
  const auto initial_dpos_balance = initial_dpos_account->balance;
  const auto initial_total = SUT->dposTotalAmountDelegated(0);

  auto redelegate_tx = std::make_shared<Transaction>(
      0, 0, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("reDelegate(address,address,uint256)", source_validator.address(),
                                               destination_validator.address(), u256(kRedelegation)),
      delegator.secret(), kDposContract, cfg.genesis.chain_id);
  const auto result = advance(
      {redelegate_tx}, {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true, .expect_to_fail = true});
  ASSERT_EQ(result->trx_receipts.size(), 1);

  const auto expected_gas = IntrinsicGas(redelegate_tx->getData(), false) + 80'000;
  TransactionReceipt expected_receipt;
  expected_receipt.status_code = 0;
  expected_receipt.gas_used = expected_gas;
  expected_receipt.cumulative_gas_used = expected_gas;

  auto assert_failed_redelegate_persists = [&](const std::shared_ptr<FinalChain>& chain) {
    const auto receipt = chain->transactionReceipt(1, 0);
    ASSERT_TRUE(receipt);
    EXPECT_EQ(receipt->status_code, 0);
    EXPECT_EQ(receipt->gas_used, expected_gas);
    EXPECT_EQ(receipt->cumulative_gas_used, expected_gas);
    EXPECT_EQ(receipt->logs.size(), 0);
    EXPECT_EQ(receipt->bloom(), expected_receipt.bloom());
    EXPECT_EQ(util::rlp_enc(*receipt), util::rlp_enc(expected_receipt));

    const auto header = chain->blockHeader(1);
    ASSERT_TRUE(header);
    EXPECT_EQ(header->gas_used, expected_gas);
    EXPECT_EQ(header->log_bloom, expected_receipt.bloom());

    const auto delegator_account = chain->getAccount(delegator.address());
    ASSERT_TRUE(delegator_account);
    EXPECT_EQ(delegator_account->nonce, 1);
    EXPECT_EQ(delegator_account->balance, u256(kDelegatorInitialBalance - kDelegation - expected_gas * kGasPrice));

    const auto dpos_account = chain->getAccount(kDposContract);
    ASSERT_TRUE(dpos_account);
    EXPECT_EQ(dpos_account->nonce, 1);
    EXPECT_EQ(dpos_account->balance, initial_dpos_balance);

    EXPECT_EQ(chain->dposTotalAmountDelegated(1), initial_total);

    const auto source_stakes = chain->dposValidatorsTotalStakes(1);
    EXPECT_EQ(get_validator_stake(source_stakes, source_validator.address()), u256(kSourceStake + kDelegation));
    EXPECT_EQ(get_validator_stake(source_stakes, destination_validator.address()), u256(kDestinationStake));
  };

  EXPECT_EQ(util::rlp_enc(result->trx_receipts[0]), util::rlp_enc(expected_receipt));
  EXPECT_EQ(result->final_chain_blk->log_bloom, expected_receipt.bloom());
  assert_failed_redelegate_persists(SUT);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 1);
  assert_failed_redelegate_persists(SUT);
}

TEST_F(FinalChainTest, native_dpos_redelegate_pre_mutation_failures_roll_back_state) {
  constexpr uint64_t kSourceStake = 10'000;
  constexpr uint64_t kSourceDelegation = 5'000;
  constexpr uint64_t kDestinationStake = 8'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kOwnerInitialBalance = 100'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  constexpr uint64_t kNonDelegatorInitialBalance = 10'000'000;
  constexpr uint64_t kGasPrice = 7;
  const addr_t kMissingValidator("0x0000000000000000000000000000000000000001");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair source_validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair destination_validator{
      dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const dev::KeyPair delegator{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};
  const dev::KeyPair non_delegator{dev::Secret("5555555555555555555555555555555555555555555555555555555555555555")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.initial_balances[non_delegator.address()] = kNonDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = 1'000;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = 1'000;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo source_validator_info{
      source_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  source_validator_info.delegations.emplace(owner.address(), kSourceStake);
  source_validator_info.delegations.emplace(delegator.address(), kSourceDelegation);
  state_api::ValidatorInfo destination_validator_info{
      destination_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  destination_validator_info.delegations.emplace(owner.address(), kDestinationStake);
  cfg.genesis.state.dpos.initial_validators = {source_validator_info, destination_validator_info};

  init();
  assume_only_toplevel_transfers = false;
  const auto initial_total = SUT->dposTotalAmountDelegated(0);
  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);

  const SharedTransactions transactions = {
      make_redelegate_tx(cfg, delegator, 0, kMissingValidator, destination_validator.address(), 1'000, kGasPrice),
      make_redelegate_tx(cfg, non_delegator, 0, source_validator.address(), destination_validator.address(), 1'000,
                         kGasPrice),
      make_redelegate_tx(cfg, delegator, 1, source_validator.address(), destination_validator.address(),
                         kSourceDelegation + 1, kGasPrice),
      make_redelegate_tx(cfg, delegator, 2, source_validator.address(), destination_validator.address(), 4'500,
                         kGasPrice),
  };
  const auto result = advance(transactions, {.expect_to_fail = true});
  ASSERT_EQ(result->trx_receipts.size(), transactions.size());
  for (const auto& receipt : result->trx_receipts) {
    EXPECT_EQ(receipt.status_code, 0);
    EXPECT_TRUE(receipt.logs.empty());
  }

  auto assert_unchanged = [&](const std::shared_ptr<FinalChain>& chain) {
    EXPECT_EQ(chain->dposTotalAmountDelegated(1), initial_total);
    const auto stakes = chain->dposValidatorsTotalStakes(1);
    EXPECT_EQ(get_validator_stake(stakes, source_validator.address()),
              get_validator_stake(initial_stakes, source_validator.address()));
    EXPECT_EQ(get_validator_stake(stakes, destination_validator.address()),
              get_validator_stake(initial_stakes, destination_validator.address()));
  };
  assert_unchanged(SUT);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  ASSERT_EQ(SUT->lastBlockNumber(), 1);
  assert_unchanged(SUT);
}

TEST_F(FinalChainTest, native_dpos_redelegate_correction_applies_only_at_fix_block) {
  constexpr uint64_t kSourceStake = 10'000;
  constexpr uint64_t kSourceDelegation = 5'000;
  constexpr uint64_t kBugAmount = 1'000;
  constexpr uint64_t kCorrection = kBugAmount;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 10'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  constexpr uint64_t kFixBlockNum = 2;

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair source_validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair delegator{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;
  cfg.genesis.state.hardforks.fix_redelegate_block_num = kFixBlockNum;
  cfg.genesis.state.hardforks.redelegations = {
      taraxa::Redelegation{source_validator.address(), delegator.address(), kCorrection}};

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo source_validator_info{
      source_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  source_validator_info.delegations.emplace(owner.address(), kSourceStake);
  source_validator_info.delegations.emplace(delegator.address(), kSourceDelegation);
  cfg.genesis.state.dpos.initial_validators = {source_validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);
  const auto initial_total = SUT->dposTotalAmountDelegated(0);
  const auto initial_stake = get_validator_stake(initial_stakes, source_validator.address());

  const auto pre_fix_tx =
      make_redelegate_tx(cfg, delegator, 0, source_validator.address(), source_validator.address(), kBugAmount, 0);
  const auto block1 = advance({pre_fix_tx}, {.dont_assume_no_logs = true});
  ASSERT_EQ(block1->trx_receipts.size(), 1);
  EXPECT_EQ(block1->trx_receipts[0].status_code, 1);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(1), initial_total);
  EXPECT_EQ(get_validator_stake(SUT->dposValidatorsTotalStakes(1), source_validator.address()),
            initial_stake + kBugAmount);

  const auto fix_block_tx =
      make_redelegate_tx(cfg, delegator, 1, source_validator.address(), source_validator.address(), kBugAmount, 0);
  const auto block2 = advance({fix_block_tx}, {.dont_assume_no_logs = true});
  ASSERT_EQ(block2->trx_receipts.size(), 1);
  EXPECT_EQ(block2->trx_receipts[0].status_code, 1);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(2), initial_total);
  EXPECT_EQ(get_validator_stake(SUT->dposValidatorsTotalStakes(2), source_validator.address()),
            initial_stake + kBugAmount);
  EXPECT_EQ(SUT->dposEligibleVoteCount(2, source_validator.address()), 1);
  EXPECT_EQ(SUT->dposEligibleTotalVoteCount(2), 1);

  const auto post_fix_tx =
      make_redelegate_tx(cfg, delegator, 2, source_validator.address(), source_validator.address(), kBugAmount, 0);
  const auto block3 = advance(
      {post_fix_tx}, {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true, .expect_to_fail = true});
  ASSERT_EQ(block3->trx_receipts.size(), 1);
  EXPECT_EQ(block3->trx_receipts[0].status_code, 0);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(3), initial_total);
  EXPECT_EQ(get_validator_stake(SUT->dposValidatorsTotalStakes(3), source_validator.address()),
            initial_stake + kBugAmount);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 3);
  EXPECT_EQ(get_validator_stake(SUT->dposValidatorsTotalStakes(3), source_validator.address()),
            initial_stake + kBugAmount);
  EXPECT_EQ(SUT->dposEligibleVoteCount(3, source_validator.address()), 1);
  EXPECT_EQ(SUT->dposEligibleTotalVoteCount(3), 1);
}

TEST_F(FinalChainTest, native_dpos_redelegate_zero_to_new_destination_pair_pre_aspen_succeeds) {
  constexpr uint64_t kSourceStake = 10'000;
  constexpr uint64_t kSourceDelegation = 5'000;
  constexpr uint64_t kDestinationStake = 8'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kRedelegation = 0;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kOwnerInitialBalance = 100'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  const auto gas_limit = TEST_TX_GAS_LIMIT;

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair source_validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair destination_validator{
      dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const dev::KeyPair delegator{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;
  cfg.genesis.state.hardforks.aspen_hf.block_num_part_one = std::numeric_limits<uint64_t>::max();
  cfg.genesis.state.hardforks.aspen_hf.block_num_part_two = std::numeric_limits<uint64_t>::max();

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo source_validator_info{
      source_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  source_validator_info.delegations.emplace(owner.address(), kSourceStake);
  source_validator_info.delegations.emplace(delegator.address(), kSourceDelegation);
  state_api::ValidatorInfo destination_validator_info{
      destination_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  destination_validator_info.delegations.emplace(owner.address(), kDestinationStake);
  cfg.genesis.state.dpos.initial_validators = {source_validator_info, destination_validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);
  const auto initial_total = SUT->dposTotalAmountDelegated(0);
  const auto initial_source_stake = get_validator_stake(initial_stakes, source_validator.address());
  const auto initial_destination_stake = get_validator_stake(initial_stakes, destination_validator.address());

  const auto redelegate_tx =
      make_redelegate_tx(cfg, delegator, 0, source_validator.address(), destination_validator.address(),
                         u256(kRedelegation), kGasPrice, gas_limit);
  const auto result = advance({redelegate_tx}, {.dont_assume_no_logs = true});
  ASSERT_EQ(result->trx_receipts.size(), 1);
  EXPECT_EQ(result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(1), initial_total);
  EXPECT_EQ(get_validator_stake(SUT->dposValidatorsTotalStakes(1), source_validator.address()), initial_source_stake);
  EXPECT_EQ(get_validator_stake(SUT->dposValidatorsTotalStakes(1), destination_validator.address()),
            initial_destination_stake);
}

TEST_F(FinalChainTest, native_dpos_redelegate_zero_at_aspen_fails) {
  constexpr uint64_t kSourceStake = 10'000;
  constexpr uint64_t kDelegation = 5'000;
  constexpr uint64_t kDestinationStake = 8'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kGasLimit = TEST_TX_GAS_LIMIT;
  constexpr uint64_t kRedelegation = 0;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kOwnerInitialBalance = 100'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair source_validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair destination_validator{
      dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const dev::KeyPair delegator{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;
  cfg.genesis.state.hardforks.aspen_hf.block_num_part_one = 1;
  cfg.genesis.state.hardforks.aspen_hf.block_num_part_two = 1;
  cfg.genesis.state.hardforks.aspen_hf.max_supply = kOwnerInitialBalance + kDelegatorInitialBalance;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo source_validator_info{
      source_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  source_validator_info.delegations.emplace(owner.address(), kSourceStake);
  source_validator_info.delegations.emplace(delegator.address(), kDelegation);
  state_api::ValidatorInfo destination_validator_info{
      destination_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  destination_validator_info.delegations.emplace(owner.address(), kDestinationStake);
  cfg.genesis.state.dpos.initial_validators = {source_validator_info, destination_validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_account = SUT->getAccount(kDposContractAddress);
  ASSERT_TRUE(initial_dpos_account);
  const auto initial_dpos_balance = initial_dpos_account->balance;
  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);
  const auto initial_total = SUT->dposTotalAmountDelegated(0);
  const auto initial_source_stake = get_validator_stake(initial_stakes, source_validator.address());
  const auto initial_destination_stake = get_validator_stake(initial_stakes, destination_validator.address());

  const auto redelegate_tx =
      make_redelegate_tx(cfg, delegator, 0, source_validator.address(), destination_validator.address(),
                         u256(kRedelegation), kGasPrice, kGasLimit);
  const auto result = advance(
      {redelegate_tx}, {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true, .expect_to_fail = true});
  ASSERT_EQ(result->trx_receipts.size(), 1);
  const auto expected_gas = IntrinsicGas(redelegate_tx->getData(), false) + 80'000;
  EXPECT_EQ(result->trx_receipts[0].status_code, 0);
  EXPECT_EQ(result->trx_receipts[0].gas_used, expected_gas);
  EXPECT_EQ(result->trx_receipts[0].logs.size(), 0);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(1), initial_total);
  EXPECT_EQ(get_validator_stake(SUT->dposValidatorsTotalStakes(1), source_validator.address()), initial_source_stake);
  EXPECT_EQ(get_validator_stake(SUT->dposValidatorsTotalStakes(1), destination_validator.address()),
            initial_destination_stake);
  EXPECT_EQ(SUT->getAccount(kDposContractAddress)->balance, initial_dpos_balance);
}

TEST_F(FinalChainTest, native_dpos_redelegate_sub_minimum_to_new_destination_pair_succeeds) {
  constexpr uint64_t kSourceStake = 10'000;
  constexpr uint64_t kSourceDelegation = 5'000;
  constexpr uint64_t kDestinationStake = 8'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kRedelegation = 1;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kOwnerInitialBalance = 100'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair source_validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair destination_validator{
      dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const dev::KeyPair delegator{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo source_validator_info{
      source_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  source_validator_info.delegations.emplace(owner.address(), kSourceStake);
  source_validator_info.delegations.emplace(delegator.address(), kSourceDelegation);
  state_api::ValidatorInfo destination_validator_info{
      destination_validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  destination_validator_info.delegations.emplace(owner.address(), kDestinationStake);
  cfg.genesis.state.dpos.initial_validators = {source_validator_info, destination_validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);
  const auto initial_total = SUT->dposTotalAmountDelegated(0);
  const auto initial_source_stake = get_validator_stake(initial_stakes, source_validator.address());
  const auto initial_destination_stake = get_validator_stake(initial_stakes, destination_validator.address());

  const auto redelegate_tx = make_redelegate_tx(cfg, delegator, 0, source_validator.address(),
                                                destination_validator.address(), u256(kRedelegation), kGasPrice);
  const auto result = advance({redelegate_tx}, {.dont_assume_no_logs = true});
  ASSERT_EQ(result->trx_receipts.size(), 1);
  const auto expected_gas = IntrinsicGas(redelegate_tx->getData(), false) + 80'000;
  EXPECT_EQ(result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(result->trx_receipts[0].gas_used, expected_gas);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(1), initial_total);
  EXPECT_EQ(get_validator_stake(SUT->dposValidatorsTotalStakes(1), source_validator.address()),
            initial_source_stake - kRedelegation);
  EXPECT_EQ(get_validator_stake(SUT->dposValidatorsTotalStakes(1), destination_validator.address()),
            initial_destination_stake + kRedelegation);
}

TEST_F(FinalChainTest, native_dpos_undelegate_v1_pre_mutation_failures_roll_back_state) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  constexpr uint64_t kNonDelegatorInitialBalance = 10'000'000;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kFailureValue = 13;
  constexpr uint64_t kContinuationValue = 1'000;
  constexpr uint64_t kContinuationGas = 21'000;
  constexpr uint64_t kGasLimit = TEST_TX_GAS_LIMIT;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair delegator{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const dev::KeyPair non_delegator{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};
  const auto missing_validator = addr_t("0x0000000000000000000000000000000000000001");

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.initial_balances[non_delegator.address()] = kNonDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  validator_info.delegations.emplace(delegator.address(), u256(kMinimumDeposit));
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  expected_blk_num = 0;
  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_account = SUT->getAccount(kDposContract);
  ASSERT_TRUE(initial_dpos_account);
  const auto initial_dpos_balance = initial_dpos_account->balance;
  const auto initial_total_delegated = SUT->dposTotalAmountDelegated(0);
  const auto initial_validator_votes = SUT->dposEligibleVoteCount(0, validator.address());
  const auto initial_total_votes = SUT->dposEligibleTotalVoteCount(0);
  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);
  const auto delegator_account_after_init = SUT->getAccount(delegator.address());
  ASSERT_TRUE(delegator_account_after_init);
  const auto delegator_balance_after_init = delegator_account_after_init->balance;
  const auto owner_account_after_init = SUT->getAccount(owner.address());
  ASSERT_TRUE(owner_account_after_init);
  const auto owner_balance_after_init = owner_account_after_init->balance;

  const auto missing_validator_calldata = util::EncodingSolidity::packFunctionCall(
      "undelegate(address,uint256)", missing_validator, u256(1'000));
  auto tx_missing_validator = std::make_shared<Transaction>(
      0, kFailureValue, kGasPrice, kGasLimit, missing_validator_calldata, delegator.secret(), kDposContract,
      cfg.genesis.chain_id);

  const auto non_delegator_calldata = util::EncodingSolidity::packFunctionCall(
      "undelegate(address,uint256)", validator.address(), u256(1));
  auto tx_non_delegator = std::make_shared<Transaction>(
      0, 0, kGasPrice, kGasLimit, non_delegator_calldata, non_delegator.secret(), kDposContract, cfg.genesis.chain_id);

  const auto tx_too_big_calldata = util::EncodingSolidity::packFunctionCall(
      "undelegate(address,uint256)", validator.address(), u256(1'001));
  auto tx_too_big = std::make_shared<Transaction>(
      1, 0, kGasPrice, kGasLimit, tx_too_big_calldata, delegator.secret(), kDposContract, cfg.genesis.chain_id);

  const auto tx_below_min_remainder_calldata = util::EncodingSolidity::packFunctionCall(
      "undelegate(address,uint256)", validator.address(), u256(1));
  auto tx_below_min_remainder =
      std::make_shared<Transaction>(2, 0, kGasPrice, kGasLimit, tx_below_min_remainder_calldata, delegator.secret(),
                                   kDposContract, cfg.genesis.chain_id);
  auto continuation = std::make_shared<Transaction>(
      3, kContinuationValue, kGasPrice, kGasLimit, bytes(), delegator.secret(), owner.address(), cfg.genesis.chain_id);

  const auto tx1_failure_gas = IntrinsicGas(missing_validator_calldata, false) + 60'000;
  const auto tx2_failure_gas = IntrinsicGas(non_delegator_calldata, false) + 60'000;
  const auto tx3_failure_gas = IntrinsicGas(tx_too_big_calldata, false) + 60'000;
  const auto tx4_failure_gas = IntrinsicGas(tx_below_min_remainder_calldata, false) + 60'000;
  const auto expected_block_gas = tx1_failure_gas + tx2_failure_gas + tx3_failure_gas + tx4_failure_gas + kContinuationGas;

  std::array<TransactionReceipt, 5> expected_receipts{};
  expected_receipts[0].status_code = 0;
  expected_receipts[0].gas_used = tx1_failure_gas;
  expected_receipts[0].cumulative_gas_used = tx1_failure_gas;

  expected_receipts[1].status_code = 0;
  expected_receipts[1].gas_used = tx2_failure_gas;
  expected_receipts[1].cumulative_gas_used = tx1_failure_gas + tx2_failure_gas;

  expected_receipts[2].status_code = 0;
  expected_receipts[2].gas_used = tx3_failure_gas;
  expected_receipts[2].cumulative_gas_used = tx1_failure_gas + tx2_failure_gas + tx3_failure_gas;

  expected_receipts[3].status_code = 0;
  expected_receipts[3].gas_used = tx4_failure_gas;
  expected_receipts[3].cumulative_gas_used = expected_receipts[2].cumulative_gas_used + tx4_failure_gas;

  expected_receipts[4].status_code = 1;
  expected_receipts[4].gas_used = kContinuationGas;
  expected_receipts[4].cumulative_gas_used = expected_receipts[3].cumulative_gas_used + kContinuationGas;

  const auto result = advance(
      {tx_missing_validator, tx_non_delegator, tx_too_big, tx_below_min_remainder, continuation},
      {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true});

  ASSERT_EQ(result->trx_receipts.size(), expected_receipts.size());
  EXPECT_EQ(result->final_chain_blk->number, 1);
  EXPECT_EQ(result->final_chain_blk->gas_used, expected_block_gas);
  EXPECT_EQ(result->final_chain_blk->log_bloom, LogBloom());

  for (size_t i = 0; i < expected_receipts.size(); ++i) {
    EXPECT_EQ(util::rlp_enc(result->trx_receipts[i]), util::rlp_enc(expected_receipts[i]));
    const auto& receipt = result->trx_receipts[i];
    EXPECT_EQ(receipt.logs.size(), 0);
    EXPECT_EQ(receipt.bloom(), expected_receipts[i].bloom());
  }

  const auto expected_header = result->final_chain_blk;
  EXPECT_EQ(expected_header->gas_used, expected_block_gas);
  EXPECT_EQ(expected_header->log_bloom, LogBloom());

  const auto delegator_account = SUT->getAccount(delegator.address());
  ASSERT_TRUE(delegator_account);
  EXPECT_EQ(delegator_account->nonce, 4);
  // tx_missing_validator carries a payable value but is expected to rollback entirely on pre-mutation failure
  const auto expected_delegator_balance =
      delegator_balance_after_init -
      (tx1_failure_gas + tx3_failure_gas + tx4_failure_gas + kContinuationGas) * kGasPrice - kContinuationValue;
  EXPECT_EQ(delegator_account->balance, expected_delegator_balance);

  const auto non_delegator_account = SUT->getAccount(non_delegator.address());
  ASSERT_TRUE(non_delegator_account);
  const auto expected_non_delegator_balance = u256(kNonDelegatorInitialBalance - tx2_failure_gas * kGasPrice);
  EXPECT_EQ(non_delegator_account->nonce, 1);
  EXPECT_EQ(non_delegator_account->balance, expected_non_delegator_balance);

  const auto owner_account = SUT->getAccount(owner.address());
  ASSERT_TRUE(owner_account);
  EXPECT_EQ(owner_account->balance, owner_balance_after_init + kContinuationValue);

  const auto dpos_account = SUT->getAccount(kDposContract);
  ASSERT_TRUE(dpos_account);
  EXPECT_EQ(dpos_account->nonce, 1);
  EXPECT_EQ(dpos_account->balance, initial_dpos_balance);

  EXPECT_EQ(SUT->dposTotalAmountDelegated(1), initial_total_delegated);
  EXPECT_EQ(SUT->dposEligibleVoteCount(1, validator.address()), initial_validator_votes);
  EXPECT_EQ(SUT->dposEligibleTotalVoteCount(1), initial_total_votes);
  const auto stakes = SUT->dposValidatorsTotalStakes(1);
  ASSERT_EQ(stakes.size(), initial_stakes.size());
  for (size_t idx = 0; idx < stakes.size(); ++idx) {
    EXPECT_EQ(stakes[idx].addr, initial_stakes[idx].addr);
    EXPECT_EQ(stakes[idx].stake, initial_stakes[idx].stake);
  }

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 1);
  const auto delegator_account_after_restart = SUT->getAccount(delegator.address());
  ASSERT_TRUE(delegator_account_after_restart);
  EXPECT_EQ(delegator_account_after_restart->nonce, 4);
  EXPECT_EQ(delegator_account_after_restart->balance, expected_delegator_balance);
  const auto non_delegator_account_after_restart = SUT->getAccount(non_delegator.address());
  ASSERT_TRUE(non_delegator_account_after_restart);
  EXPECT_EQ(non_delegator_account_after_restart->nonce, 1);
  EXPECT_EQ(non_delegator_account_after_restart->balance, expected_non_delegator_balance);
  const auto owner_account_after_restart = SUT->getAccount(owner.address());
  ASSERT_TRUE(owner_account_after_restart);
  EXPECT_EQ(owner_account_after_restart->balance, owner_balance_after_init + kContinuationValue);
  const auto dpos_account_after_restart = SUT->getAccount(kDposContract);
  ASSERT_TRUE(dpos_account_after_restart);
  EXPECT_EQ(dpos_account_after_restart->balance, initial_dpos_balance);

  EXPECT_EQ(SUT->dposTotalAmountDelegated(1), initial_total_delegated);
  EXPECT_EQ(SUT->dposEligibleVoteCount(1, validator.address()), initial_validator_votes);
  EXPECT_EQ(SUT->dposEligibleTotalVoteCount(1), initial_total_votes);
  const auto restart_stakes = SUT->dposValidatorsTotalStakes(1);
  ASSERT_EQ(restart_stakes.size(), initial_stakes.size());
  for (size_t idx = 0; idx < restart_stakes.size(); ++idx) {
    EXPECT_EQ(restart_stakes[idx].addr, initial_stakes[idx].addr);
    EXPECT_EQ(restart_stakes[idx].stake, initial_stakes[idx].stake);
  }
}

TEST_F(FinalChainTest, native_dpos_undelegate_v1_create_query_cancel_success_and_failures) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kDelegatorStake = 10'000;
  constexpr uint64_t kUndelegationAmount = 2'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kGasLimit = 100'000;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");
  const auto to_u256 = [](const bytes& value, size_t offset) {
    return u256("0x" + dev::toHex(bytes(value.begin() + offset, value.begin() + offset + 32)));
  };

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair delegator{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const dev::KeyPair other_validator{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  validator_info.delegations.emplace(delegator.address(), kDelegatorStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto expected_total = SUT->dposTotalAmountDelegated(0);
  const auto query_pending = [&](const std::shared_ptr<FinalChain>& chain, uint64_t block_num,
                                 const addr_t& request_delegator) {
    const auto response = chain->call(
        {addr_t{}, 0, kDposContract, 0, 0, 1000000,
         util::EncodingSolidity::packFunctionCall("getUndelegations(address,uint32)", request_delegator, uint32_t{0})},
        block_num);
    EXPECT_EQ(response.code_err, "");
    return response.code_retval;
  };

  const auto undelegate_tx =
      std::make_shared<Transaction>(0, 0, kGasPrice, kGasLimit,
                                    util::EncodingSolidity::packFunctionCall(
                                        "undelegate(address,uint256)", validator.address(), u256(kUndelegationAmount)),
                                    delegator.secret(), kDposContract, cfg.genesis.chain_id);
  const auto result1 = advance({undelegate_tx}, {.dont_assume_no_logs = true});
  EXPECT_EQ(result1->trx_receipts[0].status_code, 1);
  EXPECT_EQ(result1->trx_receipts[0].gas_used, IntrinsicGas(undelegate_tx->getData(), false) + 60'000);
  EXPECT_EQ(result1->trx_receipts[0].logs.size(), 1);
  const auto& created_log = result1->trx_receipts[0].logs[0];
  EXPECT_EQ(created_log.address, kDposContract);
  EXPECT_EQ(created_log.topics.size(), 3);
  EXPECT_EQ(created_log.topics[0], dev::sha3(dev::asBytes("Undelegated(address,address,uint256)")));
  EXPECT_EQ(created_log.topics[1], h256(delegator.address(), h256::AlignRight));
  EXPECT_EQ(created_log.topics[2], h256(validator.address(), h256::AlignRight));
  EXPECT_EQ(created_log.data.size(), 32);
  EXPECT_EQ(to_u256(created_log.data, 0), u256(kUndelegationAmount));
  EXPECT_EQ(result1->final_chain_blk->log_bloom, result1->trx_receipts[0].bloom());

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  const auto pending_after_create = query_pending(SUT, 1, delegator.address());
  ASSERT_GE(pending_after_create.size(), 224u);
  EXPECT_EQ(pending_after_create.size(), 224u);
  EXPECT_EQ(to_u256(pending_after_create, 32), 1);  // is_end
  EXPECT_EQ(to_u256(pending_after_create, 64), 1);  // count
  EXPECT_EQ(to_u256(pending_after_create, 96), u256(kUndelegationAmount));
  EXPECT_EQ(to_u256(pending_after_create, 192), 1);  // validator exists
  EXPECT_EQ(SUT->dposTotalAmountDelegated(1), expected_total - kUndelegationAmount);

  const auto duplicate_tx =
      std::make_shared<Transaction>(1, 0, kGasPrice, kGasLimit,
                                    util::EncodingSolidity::packFunctionCall(
                                        "undelegate(address,uint256)", validator.address(), u256(kUndelegationAmount)),
                                    delegator.secret(), kDposContract, cfg.genesis.chain_id);
  const auto confirm_missing_tx = std::make_shared<Transaction>(
      2, 0, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("confirmUndelegate(address)", other_validator.address()),
      delegator.secret(), kDposContract, cfg.genesis.chain_id);
  const auto cancel_tx = std::make_shared<Transaction>(
      3, 0, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("cancelUndelegate(address)", validator.address()), delegator.secret(),
      kDposContract, cfg.genesis.chain_id);
  const auto result2 = advance({duplicate_tx, confirm_missing_tx, cancel_tx},
                               {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true});
  ASSERT_EQ(result2->trx_receipts.size(), 3);
  EXPECT_EQ(result2->trx_receipts[0].status_code, 0);
  EXPECT_EQ(result2->trx_receipts[1].status_code, 0);
  EXPECT_EQ(result2->trx_receipts[2].status_code, 1);
  EXPECT_EQ(result2->trx_receipts[0].gas_used, IntrinsicGas(duplicate_tx->getData(), false) + 60'000);
  EXPECT_EQ(result2->trx_receipts[1].gas_used, IntrinsicGas(confirm_missing_tx->getData(), false) + 20'000);
  EXPECT_EQ(result2->trx_receipts[2].gas_used, IntrinsicGas(cancel_tx->getData(), false) + 60'000);
  EXPECT_TRUE(result2->trx_receipts[0].logs.empty());
  EXPECT_TRUE(result2->trx_receipts[1].logs.empty());
  const auto& canceled_log = result2->trx_receipts[2].logs[0];
  EXPECT_EQ(canceled_log.address, kDposContract);
  EXPECT_EQ(canceled_log.topics.size(), 3);
  EXPECT_EQ(canceled_log.topics[0], dev::sha3(dev::asBytes("UndelegateCanceled(address,address,uint256)")));
  EXPECT_EQ(canceled_log.topics[1], h256(delegator.address(), h256::AlignRight));
  EXPECT_EQ(canceled_log.topics[2], h256(validator.address(), h256::AlignRight));
  EXPECT_EQ(canceled_log.data.size(), 32);
  EXPECT_EQ(to_u256(canceled_log.data, 0), u256(kUndelegationAmount));
  EXPECT_EQ(result2->final_chain_blk->log_bloom, result2->trx_receipts[2].bloom());

  const auto pending_after_cancel = query_pending(SUT, 2, delegator.address());
  ASSERT_GE(pending_after_cancel.size(), 96u);
  EXPECT_EQ(pending_after_cancel.size(), 96u);
  EXPECT_EQ(to_u256(pending_after_cancel, 32), 1);
  EXPECT_EQ(to_u256(pending_after_cancel, 64), 0);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(2), expected_total);
}

TEST_F(FinalChainTest, native_dpos_undelegate_v1_confirm_locked_fail_then_unlock_success) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kDelegatorStake = 10'000;
  constexpr uint64_t kUndelegationAmount = 2'000;
  constexpr uint64_t kBaseDelegationLock = 9;
  constexpr uint64_t kCactiDelegationLock = 2;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kGasLimit = 100'000;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");
  const auto to_u256 = [](const bytes& value, size_t offset) {
    return u256("0x" + dev::toHex(bytes(value.begin() + offset, value.begin() + offset + 32)));
  };

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair delegator{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_locking_period = kBaseDelegationLock;
  cfg.genesis.state.hardforks.cacti_hf.delegation_locking_period = kCactiDelegationLock;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  validator_info.delegations.emplace(delegator.address(), kDelegatorStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_balance = SUT->getAccount(kDposContract)->balance;
  const auto expected_total = SUT->dposTotalAmountDelegated(0);
  const auto query_pending = [&](const std::shared_ptr<FinalChain>& chain, uint64_t block_num) {
    const auto response = chain->call({addr_t{}, 0, kDposContract, 0, 0, 1000000,
                                       util::EncodingSolidity::packFunctionCall("getUndelegations(address,uint32)",
                                                                                delegator.address(), uint32_t{0})},
                                      block_num);
    EXPECT_EQ(response.code_err, "");
    return response.code_retval;
  };

  const auto undelegate_tx =
      std::make_shared<Transaction>(0, 0, kGasPrice, kGasLimit,
                                    util::EncodingSolidity::packFunctionCall(
                                        "undelegate(address,uint256)", validator.address(), u256(kUndelegationAmount)),
                                    delegator.secret(), kDposContract, cfg.genesis.chain_id);
  const auto result1 = advance({undelegate_tx}, {.dont_assume_no_logs = true});
  EXPECT_EQ(result1->trx_receipts[0].status_code, 1);
  EXPECT_EQ(result1->trx_receipts[0].gas_used, IntrinsicGas(undelegate_tx->getData(), false) + 60'000);
  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  const auto pending_after_create = query_pending(SUT, 1);
  ASSERT_GE(pending_after_create.size(), 224u);
  EXPECT_EQ(to_u256(pending_after_create, 64), 1);
  EXPECT_EQ(to_u256(pending_after_create, 96), u256(kUndelegationAmount));
  EXPECT_EQ(to_u256(pending_after_create, 128), u256(kCactiDelegationLock + 1));

  const auto confirm_tx_locked = std::make_shared<Transaction>(
      1, 0, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("confirmUndelegate(address)", validator.address()), delegator.secret(),
      kDposContract, cfg.genesis.chain_id);
  const auto confirm_fail_result =
      advance({confirm_tx_locked}, {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true});
  EXPECT_EQ(confirm_fail_result->trx_receipts[0].status_code, 0);
  EXPECT_EQ(confirm_fail_result->trx_receipts[0].gas_used, IntrinsicGas(confirm_tx_locked->getData(), false) + 20'000);
  EXPECT_EQ(confirm_fail_result->final_chain_blk->log_bloom, LogBloom());
  const auto balance_after_failed_confirm = SUT->getAccount(delegator.address())->balance;
  const auto pending_after_failed_confirm = query_pending(SUT, 2);
  ASSERT_GE(pending_after_failed_confirm.size(), 224u);
  EXPECT_EQ(to_u256(pending_after_failed_confirm, 64), 1);

  const auto confirm_tx_success = std::make_shared<Transaction>(
      2, 0, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("confirmUndelegate(address)", validator.address()), delegator.secret(),
      kDposContract, cfg.genesis.chain_id);
  const auto confirm_success_result = advance({confirm_tx_success}, {.dont_assume_no_logs = true});
  ASSERT_EQ(confirm_success_result->trx_receipts.size(), 1);
  EXPECT_EQ(confirm_success_result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(confirm_success_result->trx_receipts[0].gas_used,
            IntrinsicGas(confirm_tx_success->getData(), false) + 20'000);
  ASSERT_EQ(confirm_success_result->trx_receipts[0].logs.size(), 1);
  const auto& success_log = confirm_success_result->trx_receipts[0].logs[0];
  EXPECT_EQ(success_log.address, kDposContract);
  EXPECT_EQ(success_log.topics[0], dev::sha3(dev::asBytes("UndelegateConfirmed(address,address,uint256)")));
  EXPECT_EQ(success_log.topics[1], h256(delegator.address(), h256::AlignRight));
  EXPECT_EQ(success_log.topics[2], h256(validator.address(), h256::AlignRight));
  EXPECT_EQ(success_log.data.size(), 32);
  EXPECT_EQ(to_u256(success_log.data, 0), u256(kUndelegationAmount));
  EXPECT_EQ(confirm_success_result->final_chain_blk->log_bloom, confirm_success_result->trx_receipts[0].bloom());

  const auto pending_after_success = query_pending(SUT, 3);
  ASSERT_GE(pending_after_success.size(), 96u);
  EXPECT_EQ(pending_after_success.size(), 96u);
  EXPECT_EQ(to_u256(pending_after_success, 64), 0);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(3), expected_total - kUndelegationAmount);
  EXPECT_EQ(SUT->getAccount(kDposContract)->balance, initial_dpos_balance - kUndelegationAmount);
  const auto confirm_gas = confirm_success_result->trx_receipts[0].gas_used;
  EXPECT_EQ(SUT->getAccount(delegator.address())->balance,
            balance_after_failed_confirm + u256(kUndelegationAmount) - u256(confirm_gas * kGasPrice));
}

TEST_F(FinalChainTest, native_dpos_register_validator_business_failures_roll_back_state) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kGasLimit = 200'000;
  constexpr uint64_t kContinuationGas = 21'000;
  constexpr uint64_t kSuccessValue = 5'000;
  constexpr uint64_t kOverMaximumStakeValue = 30'001;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  constexpr uint64_t kSenderInitialBalance = 20'000'000;
  constexpr uint16_t kInvalidCommission = 10'001;
  constexpr uint64_t kRegisterActionGas = 80'000;
  constexpr uint64_t kWrongProofValue = kMinimumDeposit;
  constexpr uint64_t kLowDepositValue = kMinimumDeposit - 1;
  constexpr uint64_t kLongEndpointValue = kMinimumDeposit;
  constexpr uint64_t kLongDescriptionValue = kMinimumDeposit;
  constexpr uint64_t kShortVrfValue = kMinimumDeposit;
  constexpr uint64_t kLongVrfValue = kMinimumDeposit;
  constexpr uint64_t kInvalidCommissionValue = kMinimumDeposit;
  constexpr uint64_t kDuplicateValue = kMinimumDeposit;
  constexpr uint64_t kOverMaximumStakeValueForGas = kOverMaximumStakeValue;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair sender{dev::Secret("3333331111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair existing_validator{
      dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair register_success_validator{
      dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};
  const dev::KeyPair register_wrong_proof_validator{
      dev::Secret("5555555555555555555555555555555555555555555555555555555555555555")};
  const dev::KeyPair register_low_deposit_validator{
      dev::Secret("6666666666666666666666666666666666666666666666666666666666666666")};
  const dev::KeyPair register_long_endpoint_validator{
      dev::Secret("7777777777777777777777777777777777777777777777777777777777777777")};
  const dev::KeyPair register_long_description_validator{
      dev::Secret("8888888888888888888888888888888888888888888888888888888888888888")};
  const dev::KeyPair register_short_vrf_validator{
      dev::Secret("9999999999999999999999999999999999999999999999999999999999999999")};
  const dev::KeyPair register_long_vrf_validator{
      dev::Secret("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")};
  const dev::KeyPair register_invalid_commission_validator{
      dev::Secret("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")};
  const dev::KeyPair register_duplicate_validator = register_success_validator;
  const dev::KeyPair register_over_maximum_stake_validator{
      dev::Secret("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[sender.address()] = kSenderInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto existing_vrf = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo existing_validator_info{
      existing_validator.address(), owner.address(), existing_vrf, 0, "", "", {}};
  existing_validator_info.delegations.emplace(owner.address(), kInitialStake);
  cfg.genesis.state.dpos.initial_validators = {existing_validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_account = SUT->getAccount(kDposContract);
  ASSERT_TRUE(initial_dpos_account);
  const auto initial_dpos_balance = initial_dpos_account->balance;
  EXPECT_EQ(initial_dpos_account->nonce, 1);
  EXPECT_EQ(initial_dpos_balance, u256(kInitialStake));
  const auto initial_block_votes = SUT->dposEligibleTotalVoteCount(0);
  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);
  ASSERT_EQ(initial_stakes.size(), 1);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(0), u256(kInitialStake));
  EXPECT_EQ(initial_stakes[0].addr, existing_validator.address());
  EXPECT_EQ(initial_stakes[0].stake, u256(kInitialStake));

  const auto default_vrf_hash = vrf_wrapper::getVrfKeyPair().first;
  const bytes default_vrf_key(default_vrf_hash.begin(), default_vrf_hash.end());
  const bytes wrong_vrf_key(default_vrf_key.begin(), default_vrf_key.begin() + 31);
  auto long_vrf_key = default_vrf_key;
  long_vrf_key.push_back(0x11);

  const auto make_proof = [](const dev::KeyPair& validator, bool wrong) {
    auto proof = dev::sign(validator.secret(), dev::sha3(validator.address())).asBytes();
    proof[64] += 27;
    if (wrong) {
      proof[0] ^= 0x01;
    }
    return proof;
  };

  auto make_register_validator_tx = [&](uint64_t nonce, const dev::KeyPair& tx_sender, const dev::KeyPair& validator,
                                        bytes proof, const bytes& vrf_key, uint16_t commission,
                                        const std::string& description, const std::string& endpoint, uint64_t value) {
    const auto calldata = util::EncodingSolidity::packFunctionCall(
        "registerValidator(address,bytes,bytes,uint16,string,string)", validator.address(), proof, vrf_key, commission,
        dev::asBytes(description), dev::asBytes(endpoint));
    return std::make_shared<Transaction>(nonce, value, kGasPrice, kGasLimit, calldata, tx_sender.secret(),
                                         kDposContract, cfg.genesis.chain_id);
  };
  auto make_continuation_tx = [&](uint64_t nonce, const addr_t& receiver, uint64_t value) {
    return std::make_shared<Transaction>(nonce, value, kGasPrice, kContinuationGas, dev::bytes(), sender.secret(),
                                         receiver, cfg.genesis.chain_id);
  };

  auto success_tx =
      make_register_validator_tx(0, sender, register_success_validator, make_proof(register_success_validator, false),
                                 default_vrf_key, 4'000, "test", "test", kSuccessValue);
  auto wrong_proof_tx = make_register_validator_tx(1, sender, register_wrong_proof_validator,
                                                   make_proof(register_wrong_proof_validator, true), default_vrf_key,
                                                   4'000, "test", "test", kWrongProofValue);
  auto low_deposit_tx = make_register_validator_tx(2, sender, register_low_deposit_validator,
                                                   make_proof(register_low_deposit_validator, false), default_vrf_key,
                                                   4'000, "test", "test", kLowDepositValue);
  auto long_endpoint_tx = make_register_validator_tx(
      3, sender, register_long_endpoint_validator, make_proof(register_long_endpoint_validator, false), default_vrf_key,
      4'000, "test", std::string(51, 'e'), kLongEndpointValue);
  auto long_description_tx = make_register_validator_tx(
      4, sender, register_long_description_validator, make_proof(register_long_description_validator, false),
      default_vrf_key, 4'000, std::string(101, 'd'), "test", kLongDescriptionValue);
  auto short_vrf_tx = make_register_validator_tx(5, sender, register_short_vrf_validator,
                                                 make_proof(register_short_vrf_validator, false), wrong_vrf_key, 4'000,
                                                 "test", "test", kShortVrfValue);
  auto long_vrf_tx =
      make_register_validator_tx(6, sender, register_long_vrf_validator, make_proof(register_long_vrf_validator, false),
                                 long_vrf_key, 4'000, "test", "test", kLongVrfValue);
  auto invalid_commission_tx = make_register_validator_tx(
      7, sender, register_invalid_commission_validator, make_proof(register_invalid_commission_validator, false),
      default_vrf_key, kInvalidCommission, "test", "test", kInvalidCommissionValue);
  auto duplicate_tx = make_register_validator_tx(8, sender, register_duplicate_validator,
                                                 make_proof(register_duplicate_validator, false), default_vrf_key,
                                                 4'000, "test", "test", kDuplicateValue);
  auto overmax_tx = make_register_validator_tx(9, sender, register_over_maximum_stake_validator,
                                               make_proof(register_over_maximum_stake_validator, false),
                                               default_vrf_key, 4'000, "test", "test", kOverMaximumStakeValueForGas);
  const uint64_t success_gas = IntrinsicGas(success_tx->getData(), false) + kRegisterActionGas;
  const uint64_t wrong_proof_gas = IntrinsicGas(wrong_proof_tx->getData(), false) + kRegisterActionGas;
  const uint64_t low_deposit_gas = IntrinsicGas(low_deposit_tx->getData(), false) + kRegisterActionGas;
  const uint64_t long_endpoint_gas = IntrinsicGas(long_endpoint_tx->getData(), false) + kRegisterActionGas;
  const uint64_t long_description_gas = IntrinsicGas(long_description_tx->getData(), false) + kRegisterActionGas;
  const uint64_t short_vrf_gas = IntrinsicGas(short_vrf_tx->getData(), false) + kRegisterActionGas;
  const uint64_t long_vrf_gas = IntrinsicGas(long_vrf_tx->getData(), false) + kRegisterActionGas;
  const uint64_t invalid_commission_gas = IntrinsicGas(invalid_commission_tx->getData(), false) + kRegisterActionGas;
  const uint64_t duplicate_gas = IntrinsicGas(duplicate_tx->getData(), false) + kRegisterActionGas;
  const uint64_t overmax_gas = IntrinsicGas(overmax_tx->getData(), false) + kRegisterActionGas;
  const uint64_t register_gas = success_gas + wrong_proof_gas + low_deposit_gas + long_endpoint_gas +
                                long_description_gas + short_vrf_gas + long_vrf_gas + invalid_commission_gas +
                                duplicate_gas + overmax_gas;
  const uint64_t committed_cost_before_continuation = (register_gas + kContinuationGas) * kGasPrice + kSuccessValue;
  ASSERT_GT(kSenderInitialBalance, committed_cost_before_continuation + 1);
  const uint64_t continuation_value = kSenderInitialBalance - committed_cost_before_continuation - 1;
  ASSERT_GT(continuation_value, kLowDepositValue);
  auto continuation_tx = make_continuation_tx(10, owner.address(), continuation_value);
  const uint64_t expected_block_gas = success_gas + wrong_proof_gas + low_deposit_gas + long_endpoint_gas +
                                      long_description_gas + short_vrf_gas + long_vrf_gas + invalid_commission_gas +
                                      duplicate_gas + overmax_gas + kContinuationGas;

  const auto result =
      advance({success_tx, wrong_proof_tx, low_deposit_tx, long_endpoint_tx, long_description_tx, short_vrf_tx,
               long_vrf_tx, invalid_commission_tx, duplicate_tx, overmax_tx, continuation_tx},
              {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true});
  ASSERT_EQ(result->trx_receipts.size(), 10 + 1);
  const auto& receipts = result->trx_receipts;

  std::array<uint64_t, 11> expected_gas_used{
      success_gas,  wrong_proof_gas,        low_deposit_gas, long_endpoint_gas, long_description_gas, short_vrf_gas,
      long_vrf_gas, invalid_commission_gas, duplicate_gas,   overmax_gas,       kContinuationGas};
  std::array<uint64_t, 11> expected_cumulative_gas{};
  const uint64_t expected_receipt0_cumulative = success_gas;
  const uint64_t expected_receipt1_cumulative = expected_receipt0_cumulative + wrong_proof_gas;
  const uint64_t expected_receipt2_cumulative = expected_receipt1_cumulative + low_deposit_gas;
  const uint64_t expected_receipt3_cumulative = expected_receipt2_cumulative + long_endpoint_gas;
  const uint64_t expected_receipt4_cumulative = expected_receipt3_cumulative + long_description_gas;
  const uint64_t expected_receipt5_cumulative = expected_receipt4_cumulative + short_vrf_gas;
  const uint64_t expected_receipt6_cumulative = expected_receipt5_cumulative + long_vrf_gas;
  const uint64_t expected_receipt7_cumulative = expected_receipt6_cumulative + invalid_commission_gas;
  const uint64_t expected_receipt8_cumulative = expected_receipt7_cumulative + duplicate_gas;
  const uint64_t expected_receipt9_cumulative = expected_receipt8_cumulative + overmax_gas;
  const uint64_t expected_receipt10_cumulative = expected_receipt9_cumulative + kContinuationGas;
  expected_cumulative_gas = {expected_receipt0_cumulative, expected_receipt1_cumulative, expected_receipt2_cumulative,
                             expected_receipt3_cumulative, expected_receipt4_cumulative, expected_receipt5_cumulative,
                             expected_receipt6_cumulative, expected_receipt7_cumulative, expected_receipt8_cumulative,
                             expected_receipt9_cumulative, expected_receipt10_cumulative};
  for (size_t idx = 0; idx < receipts.size(); ++idx) {
    EXPECT_EQ(receipts[idx].gas_used, expected_gas_used[idx]);
    EXPECT_EQ(receipts[idx].cumulative_gas_used, expected_cumulative_gas[idx]);
  }

  bytes delegated_amount(32, 0);
  delegated_amount[30] = 0x13;
  delegated_amount[31] = 0x88;
  const LogEntry registered_log{kDposContract,
                                {dev::sha3(dev::asBytes("ValidatorRegistered(address)")),
                                 h256(register_success_validator.address(), h256::AlignRight)},
                                bytes()};
  const LogEntry delegated_log{
      kDposContract,
      {dev::sha3(dev::asBytes("Delegated(address,address,uint256)")), h256(sender.address(), h256::AlignRight),
       h256(register_success_validator.address(), h256::AlignRight)},
      delegated_amount};

  TransactionReceipt expected_success;
  expected_success.status_code = 1;
  expected_success.gas_used = expected_gas_used[0];
  expected_success.cumulative_gas_used = expected_receipt0_cumulative;
  expected_success.logs = {registered_log, delegated_log};

  TransactionReceipt expected_failed;
  expected_failed.status_code = 0;
  expected_failed.logs = {};
  expected_failed.cumulative_gas_used = expected_receipt0_cumulative;
  expected_failed.gas_used = expected_gas_used[1];
  const auto expected_registered_bloom = expected_success.bloom();
  TransactionReceipt expected_continuation;
  expected_continuation.status_code = 1;
  expected_continuation.gas_used = expected_gas_used[10];
  expected_continuation.cumulative_gas_used = expected_cumulative_gas[10];
  EXPECT_EQ(util::rlp_enc(result->trx_receipts[0]), util::rlp_enc(expected_success));
  EXPECT_EQ(receipts[0].status_code, expected_success.status_code);
  EXPECT_EQ(receipts[0].logs.size(), 2);
  EXPECT_EQ(receipts[0].gas_used, expected_success.gas_used);
  EXPECT_EQ(receipts[0].cumulative_gas_used, expected_success.cumulative_gas_used);
  EXPECT_EQ(result->final_chain_blk->log_bloom, expected_registered_bloom);

  for (size_t idx = 1; idx < receipts.size(); ++idx) {
    if (idx == 10) {
      EXPECT_EQ(util::rlp_enc(receipts[idx]), util::rlp_enc(expected_continuation));
      EXPECT_EQ(receipts[idx].status_code, expected_continuation.status_code);
      EXPECT_EQ(receipts[idx].logs.size(), 0);
      EXPECT_EQ(receipts[idx].gas_used, expected_continuation.gas_used);
      EXPECT_EQ(receipts[idx].cumulative_gas_used, expected_continuation.cumulative_gas_used);
      EXPECT_EQ(receipts[idx].bloom(), expected_continuation.bloom());
    } else {
      expected_failed.gas_used = expected_gas_used[idx];
      expected_failed.cumulative_gas_used = expected_cumulative_gas[idx];
      EXPECT_EQ(util::rlp_enc(receipts[idx]), util::rlp_enc(expected_failed));
      EXPECT_EQ(receipts[idx].status_code, expected_failed.status_code);
      EXPECT_EQ(receipts[idx].logs.size(), 0);
      EXPECT_EQ(receipts[idx].bloom(), expected_failed.bloom());
      EXPECT_EQ(receipts[idx].gas_used, expected_failed.gas_used);
      EXPECT_EQ(receipts[idx].cumulative_gas_used, expected_failed.cumulative_gas_used);
    }
  }

  EXPECT_EQ(result->final_chain_blk->gas_used, expected_block_gas);
  EXPECT_EQ(result->final_chain_blk->gas_used, expected_cumulative_gas[10]);

  auto assert_failed_persists = [&](const std::shared_ptr<FinalChain>& chain, uint64_t block_num) {
    for (size_t idx : {1, 2, 3, 4, 5, 6, 7, 8, 9}) {
      const auto receipt = chain->transactionReceipt(block_num, idx);
      ASSERT_TRUE(receipt);
      EXPECT_EQ(receipt->status_code, 0);
      EXPECT_EQ(receipt->logs.size(), 0);
    }
    const auto continuation_receipt = chain->transactionReceipt(block_num, 10);
    ASSERT_TRUE(continuation_receipt);
    EXPECT_EQ(continuation_receipt->status_code, expected_continuation.status_code);
    EXPECT_EQ(continuation_receipt->gas_used, expected_continuation.gas_used);
    EXPECT_EQ(continuation_receipt->cumulative_gas_used, expected_continuation.cumulative_gas_used);
    EXPECT_EQ(continuation_receipt->logs.size(), 0);

    const auto success_receipt = chain->transactionReceipt(block_num, 0);
    ASSERT_TRUE(success_receipt);
    EXPECT_EQ(util::rlp_enc(*success_receipt), util::rlp_enc(expected_success));
    EXPECT_EQ(success_receipt->status_code, 1);
    EXPECT_EQ(success_receipt->gas_used, receipts[0].gas_used);
    ASSERT_EQ(success_receipt->logs.size(), 2);
    EXPECT_EQ(success_receipt->logs[0].address, registered_log.address);
    EXPECT_EQ(success_receipt->logs[0].topics, registered_log.topics);
    EXPECT_EQ(success_receipt->logs[0].data, registered_log.data);
    EXPECT_EQ(success_receipt->logs[1].address, delegated_log.address);
    EXPECT_EQ(success_receipt->logs[1].topics, delegated_log.topics);
    EXPECT_EQ(success_receipt->logs[1].data, delegated_log.data);

    EXPECT_EQ(success_receipt->cumulative_gas_used, receipts[0].gas_used);
    EXPECT_EQ(success_receipt->bloom(), expected_registered_bloom);
    EXPECT_EQ(success_receipt->gas_used, success_gas);
    EXPECT_EQ(success_receipt->cumulative_gas_used, expected_receipt0_cumulative);

    const auto header = chain->blockHeader(block_num);
    ASSERT_TRUE(header);
    EXPECT_EQ(header->gas_used, expected_block_gas);
    EXPECT_EQ(header->log_bloom, expected_registered_bloom);

    const auto sender_account = chain->getAccount(sender.address());
    ASSERT_TRUE(sender_account);
    EXPECT_EQ(sender_account->nonce, 11);
    const auto total_tx_gas = expected_block_gas;
    EXPECT_EQ(sender_account->balance,
              u256(kSenderInitialBalance - total_tx_gas * kGasPrice - (kSuccessValue + continuation_value)));

    const auto owner_account = chain->getAccount(owner.address());
    ASSERT_TRUE(owner_account);
    EXPECT_EQ(owner_account->balance, u256(kOwnerInitialBalance - kInitialStake + continuation_value));

    const auto dpos_account = chain->getAccount(kDposContract);
    ASSERT_TRUE(dpos_account);
    EXPECT_EQ(dpos_account->nonce, 1);
    EXPECT_EQ(dpos_account->balance, u256(kInitialStake + kSuccessValue));

    EXPECT_EQ(chain->dposTotalAmountDelegated(block_num), u256(kInitialStake + kSuccessValue));
    const auto stakes = chain->dposValidatorsTotalStakes(block_num);
    EXPECT_EQ(stakes.size(), 2);
    auto find_stake = [&](const addr_t& target) {
      for (const auto& item : stakes) {
        if (item.addr == target) {
          return item.stake;
        }
      }
      ADD_FAILURE() << "validator stake not found";
      return u256(0);
    };
    EXPECT_EQ(find_stake(existing_validator.address()), u256(kInitialStake));
    EXPECT_EQ(find_stake(register_success_validator.address()), u256(kSuccessValue));
    EXPECT_EQ(chain->dposEligibleVoteCount(block_num, existing_validator.address()), initial_block_votes);
    EXPECT_EQ(chain->dposEligibleVoteCount(block_num, register_success_validator.address()), kSuccessValue / kVoteStep);
    EXPECT_EQ(chain->dposEligibleTotalVoteCount(block_num), initial_block_votes + (kSuccessValue / kVoteStep));
  };

  assert_failed_persists(SUT, 1);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 1);
  assert_failed_persists(SUT, 1);
}

TEST_F(FinalChainTest, native_dpos_claim_rewards_from_sender_without_delegation_rolls_back_state) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kGasLimit = 100'000;
  constexpr uint64_t kExpectedGas = 61'464;
  constexpr uint64_t kContinuationGas = 21'000;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  constexpr uint64_t kSenderInitialBalance = 10'000'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");
  const addr_t kValidator("0x0000000000000000000000000000000000000001");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair sender{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[sender.address()] = kSenderInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{kValidator, owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_account = SUT->getAccount(kDposContract);
  ASSERT_TRUE(initial_dpos_account);
  const auto initial_dpos_balance = initial_dpos_account->balance;
  const auto initial_eligible_votes = SUT->dposEligibleVoteCount(0, kValidator);
  const auto initial_total_votes = SUT->dposEligibleTotalVoteCount(0);
  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);
  ASSERT_EQ(initial_stakes.size(), 1u);
  const auto initial_total_delegated = SUT->dposTotalAmountDelegated(0);

  const auto calldata = dev::fromHex("ef5cfb8c0000000000000000000000000000000000000000000000000000000000000001");
  ASSERT_EQ(calldata.size(), 36);
  EXPECT_EQ(bytes(calldata.begin(), calldata.begin() + 4), dev::fromHex("ef5cfb8c"));
  EXPECT_EQ(bytes(calldata.begin() + 4, calldata.end()),
            dev::fromHex("0000000000000000000000000000000000000000000000000000000000000001"));

  auto claim_trx = std::make_shared<Transaction>(0, 0, kGasPrice, kGasLimit, calldata, sender.secret(), kDposContract,
                                                 cfg.genesis.chain_id);
  auto continuation_tx =
      std::make_shared<Transaction>(1, 0, kGasPrice, kGasLimit, dev::bytes(), sender.secret(), sender.address());
  const auto claim_result =
      advance({claim_trx, continuation_tx}, {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true});
  ASSERT_EQ(claim_result->trx_receipts.size(), 2);

  TransactionReceipt expected_receipt;
  expected_receipt.status_code = 0;
  expected_receipt.gas_used = kExpectedGas;
  expected_receipt.cumulative_gas_used = kExpectedGas;

  TransactionReceipt continuation_receipt_expected;
  continuation_receipt_expected.status_code = 1;
  continuation_receipt_expected.gas_used = kContinuationGas;
  continuation_receipt_expected.cumulative_gas_used = kExpectedGas + continuation_receipt_expected.gas_used;

  const auto assert_failed_claim_state = [&](const std::shared_ptr<FinalChain>& chain,
                                             const u256& expected_sender_balance, uint64_t block_num,
                                             uint64_t expected_sender_nonce, uint64_t expected_block_gas) {
    const auto claim_receipt = chain->transactionReceipt(block_num, 0);
    ASSERT_TRUE(claim_receipt);
    EXPECT_EQ(util::rlp_enc(*claim_receipt), util::rlp_enc(expected_receipt));
    EXPECT_EQ(claim_receipt->status_code, 0);
    EXPECT_EQ(claim_receipt->gas_used, kExpectedGas);
    EXPECT_EQ(claim_receipt->cumulative_gas_used, kExpectedGas);
    EXPECT_EQ(claim_receipt->logs.size(), 0);
    EXPECT_EQ(claim_receipt->bloom(), LogBloom());

    const auto continuation_receipt = chain->transactionReceipt(block_num, 1);
    ASSERT_TRUE(continuation_receipt);
    EXPECT_EQ(continuation_receipt->status_code, continuation_receipt_expected.status_code);
    EXPECT_EQ(continuation_receipt->gas_used, continuation_receipt_expected.gas_used);
    EXPECT_EQ(continuation_receipt->cumulative_gas_used, continuation_receipt_expected.cumulative_gas_used);
    EXPECT_EQ(continuation_receipt->logs.size(), 0);
    EXPECT_EQ(continuation_receipt->bloom(), LogBloom());

    const auto header = chain->blockHeader(block_num);
    ASSERT_TRUE(header);
    EXPECT_EQ(header->gas_used, expected_block_gas);
    EXPECT_EQ(header->log_bloom, LogBloom());

    const auto sender_account = chain->getAccount(sender.address());
    ASSERT_TRUE(sender_account);
    EXPECT_EQ(sender_account->nonce, expected_sender_nonce);
    EXPECT_EQ(sender_account->balance, expected_sender_balance);

    const auto dpos_account = chain->getAccount(kDposContract);
    ASSERT_TRUE(dpos_account);
    EXPECT_EQ(dpos_account->nonce, 1);
    EXPECT_EQ(dpos_account->balance, initial_dpos_balance);

    EXPECT_EQ(chain->dposTotalAmountDelegated(block_num), initial_total_delegated);

    const auto stakes = chain->dposValidatorsTotalStakes(block_num);
    ASSERT_EQ(stakes.size(), 1u);
    EXPECT_EQ(stakes[0].addr, kValidator);
    EXPECT_EQ(stakes[0].stake, initial_stakes[0].stake);

    EXPECT_EQ(chain->dposEligibleVoteCount(block_num, kValidator), initial_eligible_votes);
    EXPECT_EQ(chain->dposEligibleTotalVoteCount(block_num), initial_total_votes);
  };

  const auto sender_expected_balance_after_block =
      u256(kSenderInitialBalance - (kExpectedGas + continuation_receipt_expected.gas_used) * kGasPrice);
  const auto block_gas = kExpectedGas + continuation_receipt_expected.gas_used;

  EXPECT_EQ(util::rlp_enc(claim_result->trx_receipts[0]), util::rlp_enc(expected_receipt));
  EXPECT_EQ(util::rlp_enc(claim_result->trx_receipts[1]), util::rlp_enc(continuation_receipt_expected));
  EXPECT_EQ(claim_result->final_chain_blk->gas_used, block_gas);
  EXPECT_EQ(claim_result->final_chain_blk->number, 1);
  EXPECT_EQ(claim_result->final_chain_blk->log_bloom, LogBloom());
  EXPECT_EQ(claim_result->trx_receipts[0].status_code, expected_receipt.status_code);
  EXPECT_EQ(claim_result->trx_receipts[1].status_code, continuation_receipt_expected.status_code);
  assert_failed_claim_state(SUT, sender_expected_balance_after_block, 1, 2, block_gas);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 1);
  assert_failed_claim_state(
      SUT, u256(kSenderInitialBalance - kExpectedGas * kGasPrice - continuation_receipt_expected.gas_used * kGasPrice),
      1, 2, block_gas);
}

TEST_F(FinalChainTest, native_dpos_claim_commission_rewards_pays_and_retains_pending_validator) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 1'000'000'000;
  constexpr uint64_t kActionGasPrice = 0;
  constexpr uint64_t kGasLimit = 200'000;
  constexpr uint64_t kOwnerInitialBalance = 21'000;
  constexpr uint64_t kSenderInitialBalance = 10'000'000'000'000'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair& validator = dag_proposer_keys;
  const dev::KeyPair sender{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const dev::KeyPair receiver{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};
  const auto to_u256 = [](const bytes& value, size_t offset) {
    return u256("0x" + dev::toHex(bytes(value.begin() + offset, value.begin() + offset + 32)));
  };

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[sender.address()] = kSenderInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_locking_period = 9;
  cfg.genesis.state.hardforks.magnolia_hf.block_num = 2;
  cfg.genesis.state.hardforks.phalaenopsis_hf_block_num = 2;
  cfg.genesis.state.hardforks.cornus_hf.block_num = 1;
  cfg.genesis.state.dpos.delegation_delay = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  const auto pbft_vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo pbft_validator_info{pbft_proposer_keys.address(), owner.address(), pbft_vrf_public_key, 0,
                                               "", "", {}};
  pbft_validator_info.delegations.emplace(owner.address(), kInitialStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info, pbft_validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto query_validator = [&](const std::shared_ptr<FinalChain>& chain, const addr_t& who) {
    return chain->call({
        addr_t{},
        kGasPrice,
        kDposContract,
        0,
        0,
        1000000,
        util::EncodingSolidity::packFunctionCall("getValidator(address)", who),
    });
  };
  const auto validator_exists = [&](const std::shared_ptr<FinalChain>& chain, const addr_t& who) {
    try {
      return query_validator(chain, who).code_err.empty();
    } catch (const std::exception&) {
      return false;
    }
  };
  const auto get_commission_rewards = [&](const std::shared_ptr<FinalChain>& chain, const addr_t& who) -> u256 {
    const auto response = query_validator(chain, who);
    if (!response.code_err.empty()) {
      return u256{0};
    }
    EXPECT_GE(response.code_retval.size(), 96);
    if (response.code_retval.size() < 96) {
      return 0;
    }
    const auto hex_commission = "0x" + dev::toHex(bytes(response.code_retval.begin() + 64, response.code_retval.begin() + 96));
    return u256(hex_commission);
  };
  const auto reward_tx = std::make_shared<Transaction>(0, 0, kGasPrice, kGasLimit, dev::bytes(), sender.secret(),
                                                      receiver.address(), cfg.genesis.chain_id);
  const auto reward_result = advance({reward_tx});
  EXPECT_EQ(reward_result->trx_receipts.size(), 1);
  EXPECT_EQ(reward_result->trx_receipts[0].status_code, 1);
  const auto magnolia_reward_tx = std::make_shared<Transaction>(1, 0, kGasPrice, kGasLimit, dev::bytes(),
                                                               sender.secret(), receiver.address(),
                                                               cfg.genesis.chain_id);
  const auto magnolia_reward_result = advance({magnolia_reward_tx});
  ASSERT_EQ(magnolia_reward_result->trx_receipts.size(), 1);
  ASSERT_EQ(magnolia_reward_result->trx_receipts[0].status_code, 1);
  const auto reward_gas_used = magnolia_reward_result->trx_receipts[0].gas_used;
  const auto initial_commission = get_commission_rewards(SUT, validator.address());
  EXPECT_EQ(initial_commission, u256(reward_gas_used * kGasPrice));
  const auto undelegate_tx =
      std::make_shared<Transaction>(0, 0, kActionGasPrice, kGasLimit,
                                    util::EncodingSolidity::packFunctionCall("undelegateV2(address,uint256)",
                                                                             validator.address(), u256(kInitialStake)),
                                    owner.secret(), kDposContract, cfg.genesis.chain_id);
  const auto undelegate_result = advance({undelegate_tx}, {.dont_assume_no_logs = true});
  EXPECT_EQ(undelegate_result->trx_receipts.size(), 1);
  EXPECT_EQ(undelegate_result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(undelegate_result->trx_receipts[0].gas_used, IntrinsicGas(undelegate_tx->getData(), false) + 60'000);
  const auto validator_state_after_undelegate = query_validator(SUT, validator.address()).code_retval;
  ASSERT_GE(validator_state_after_undelegate.size(), 192u);
  EXPECT_EQ(to_u256(validator_state_after_undelegate, 32), u256(0));
  EXPECT_EQ(to_u256(validator_state_after_undelegate, 64), initial_commission);
  EXPECT_EQ(to_u256(validator_state_after_undelegate, 160), u256(1));

  const auto dpos_balance_before_claim = SUT->getAccount(kDposContract)->balance;
  const auto owner_balance_before_claim = SUT->getAccount(owner.address())->balance;

  auto claim_tx = std::make_shared<Transaction>(
      1, 0, kActionGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("claimCommissionRewards(address)", validator.address()), owner.secret(),
      kDposContract, cfg.genesis.chain_id);
  const uint64_t claim_tx_expected_gas = IntrinsicGas(claim_tx->getData(), false) + 20'000;
  const auto claim_result = advance({claim_tx}, {.dont_assume_no_logs = true});
  ASSERT_EQ(claim_result->trx_receipts.size(), 1);
  EXPECT_EQ(claim_result->final_chain_blk->number, 4);
  EXPECT_EQ(claim_result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(claim_result->trx_receipts[0].gas_used, claim_tx_expected_gas);
  EXPECT_EQ(claim_result->final_chain_blk->gas_used, claim_tx_expected_gas);
  ASSERT_EQ(claim_result->trx_receipts[0].logs.size(), 1u);
  EXPECT_EQ(claim_result->final_chain_blk->log_bloom, claim_result->trx_receipts[0].bloom());
  EXPECT_EQ(claim_result->trx_receipts[0].logs[0].address, kDposContract);
  EXPECT_EQ(claim_result->trx_receipts[0].logs[0].topics[0],
            dev::sha3(dev::asBytes("CommissionRewardsClaimed(address,address,uint256)")));
  EXPECT_EQ(claim_result->trx_receipts[0].logs[0].topics[1], h256(owner.address(), h256::AlignRight));
  EXPECT_EQ(claim_result->trx_receipts[0].logs[0].topics[2], h256(validator.address(), h256::AlignRight));
  EXPECT_EQ(claim_result->trx_receipts[0].logs[0].data.size(), 32);
  EXPECT_EQ(to_u256(claim_result->trx_receipts[0].logs[0].data, 0), initial_commission);

  const auto expected_owner_balance_after_claim =
      owner_balance_before_claim + initial_commission - u256(claim_tx_expected_gas) * kActionGasPrice;
  const auto expected_dpos_balance_after_claim = dpos_balance_before_claim - initial_commission;
  EXPECT_EQ(SUT->getAccount(owner.address())->balance, expected_owner_balance_after_claim);
  EXPECT_EQ(SUT->getAccount(kDposContract)->balance, expected_dpos_balance_after_claim);
  EXPECT_TRUE(validator_exists(SUT, validator.address()));
  const auto validator_state_after_claim = query_validator(SUT, validator.address()).code_retval;
  ASSERT_GE(validator_state_after_claim.size(), 192u);
  EXPECT_EQ(to_u256(validator_state_after_claim, 32), u256(0));
  EXPECT_EQ(to_u256(validator_state_after_claim, 64), u256(0));
  EXPECT_EQ(to_u256(validator_state_after_claim, 160), u256(1));
  EXPECT_EQ(SUT->dposTotalAmountDelegated(4), u256(kInitialStake));
  EXPECT_EQ(SUT->dposValidatorsTotalStakes(4).size(), 2u);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 4);
  EXPECT_TRUE(validator_exists(SUT, validator.address()));
  const auto restarted_validator_state = query_validator(SUT, validator.address()).code_retval;
  ASSERT_GE(restarted_validator_state.size(), 192u);
  EXPECT_EQ(to_u256(restarted_validator_state, 32), u256(0));
  EXPECT_EQ(to_u256(restarted_validator_state, 64), u256(0));
  EXPECT_EQ(to_u256(restarted_validator_state, 160), u256(1));
  EXPECT_EQ(SUT->dposTotalAmountDelegated(4), u256(kInitialStake));
  EXPECT_EQ(SUT->dposValidatorsTotalStakes(4).size(), 2u);
}

TEST_F(FinalChainTest, native_dpos_undelegate_v2_premagnolia_deletes_validator_and_confirms_custody) {
  constexpr uint64_t kStake = 10'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 0;
  constexpr uint64_t kGasLimit = 200'000;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const auto to_u256 = [](const bytes& value, size_t offset) {
    return u256("0x" + dev::toHex(bytes(value.begin() + offset, value.begin() + offset + 32)));
  };

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_locking_period = 9;
  cfg.genesis.state.hardforks.cornus_hf.block_num = 1;
  cfg.genesis.state.hardforks.cornus_hf.delegation_locking_period = 1;
  cfg.genesis.state.hardforks.cacti_hf.block_num = 10;
  cfg.genesis.state.hardforks.magnolia_hf.block_num = 10;
  cfg.genesis.state.dpos.delegation_delay = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto validator_exists = [&](const std::shared_ptr<FinalChain>& chain) {
    try {
      const auto response = chain->call({
          addr_t{},
          kGasPrice,
          kDposContract,
          0,
          0,
          1000000,
          util::EncodingSolidity::packFunctionCall("getValidator(address)", validator.address()),
      });
      return response.code_err.empty();
    } catch (const std::exception&) {
      return false;
    }
  };

  const auto undelegate_tx = std::make_shared<Transaction>(
      0, 0, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("undelegateV2(address,uint256)", validator.address(), u256(kStake)),
      owner.secret(), kDposContract, cfg.genesis.chain_id);
  const auto undelegate_result = advance({undelegate_tx}, {.dont_assume_no_logs = true});
  ASSERT_EQ(undelegate_result->trx_receipts.size(), 1);
  const auto undelegate_gas = IntrinsicGas(undelegate_tx->getData(), false) + 60'000;
  EXPECT_EQ(undelegate_result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(undelegate_result->trx_receipts[0].gas_used, undelegate_gas);
  ASSERT_EQ(undelegate_result->trx_receipts[0].logs.size(), 1u);
  const auto& undelegate_log = undelegate_result->trx_receipts[0].logs[0];
  EXPECT_EQ(undelegate_log.topics[0],
            dev::sha3(dev::asBytes("UndelegatedV2(address,address,uint64,uint256)")));
  EXPECT_EQ(undelegate_log.topics[1], h256(owner.address(), h256::AlignRight));
  EXPECT_EQ(undelegate_log.topics[2], h256(validator.address(), h256::AlignRight));
  EXPECT_EQ(u256("0x" + dev::toHex(undelegate_log.topics[3].asBytes())), u256(1));
  EXPECT_EQ(to_u256(undelegate_log.data, 0), u256(kStake));
  EXPECT_EQ(undelegate_result->final_chain_blk->log_bloom, undelegate_result->trx_receipts[0].bloom());
  EXPECT_FALSE(validator_exists(SUT));
  EXPECT_EQ(SUT->dposTotalAmountDelegated(1), u256(0));
  EXPECT_EQ(SUT->dposValidatorsTotalStakes(1).size(), 0u);

  const auto pending = SUT->call({
      addr_t{},
      kGasPrice,
      kDposContract,
      0,
      0,
      1000000,
      util::EncodingSolidity::packFunctionCall("getUndelegationV2(address,address,uint64)", owner.address(),
                                               validator.address(), uint64_t{1}),
  });
  ASSERT_TRUE(pending.code_err.empty());
  ASSERT_GE(pending.code_retval.size(), 160u);
  EXPECT_EQ(to_u256(pending.code_retval, 0), u256(kStake));
  EXPECT_EQ(to_u256(pending.code_retval, 32), u256(2));
  EXPECT_EQ(to_u256(pending.code_retval, 64), u256("0x" + validator.address().toString()));
  EXPECT_EQ(to_u256(pending.code_retval, 96), u256(0));
  EXPECT_EQ(to_u256(pending.code_retval, 128), u256(1));

  const auto owner_balance_before_confirm = SUT->getAccount(owner.address())->balance;
  const auto dpos_balance_before_confirm = SUT->getAccount(kDposContract)->balance;
  const auto confirm_tx = std::make_shared<Transaction>(
      1, 0, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("confirmUndelegateV2(address,uint64)", validator.address(), uint64_t{1}),
      owner.secret(), kDposContract, cfg.genesis.chain_id);
  const auto confirm_result = advance({confirm_tx}, {.dont_assume_no_logs = true});
  ASSERT_EQ(confirm_result->trx_receipts.size(), 1);
  const auto confirm_gas = IntrinsicGas(confirm_tx->getData(), false) + 20'000;
  EXPECT_EQ(confirm_result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(confirm_result->trx_receipts[0].gas_used, confirm_gas);
  ASSERT_EQ(confirm_result->trx_receipts[0].logs.size(), 1u);
  const auto& confirm_log = confirm_result->trx_receipts[0].logs[0];
  EXPECT_EQ(confirm_log.topics[0],
            dev::sha3(dev::asBytes("UndelegateConfirmedV2(address,address,uint64,uint256)")));
  EXPECT_EQ(confirm_log.topics[1], h256(owner.address(), h256::AlignRight));
  EXPECT_EQ(confirm_log.topics[2], h256(validator.address(), h256::AlignRight));
  EXPECT_EQ(u256("0x" + dev::toHex(confirm_log.topics[3].asBytes())), u256(1));
  EXPECT_EQ(to_u256(confirm_log.data, 0), u256(kStake));
  EXPECT_EQ(confirm_result->final_chain_blk->log_bloom, confirm_result->trx_receipts[0].bloom());
  EXPECT_EQ(SUT->getAccount(owner.address())->balance, owner_balance_before_confirm + kStake);
  EXPECT_EQ(SUT->getAccount(kDposContract)->balance, dpos_balance_before_confirm - kStake);
  EXPECT_FALSE(validator_exists(SUT));

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 2);
  EXPECT_FALSE(validator_exists(SUT));
  EXPECT_EQ(SUT->getAccount(owner.address())->balance, owner_balance_before_confirm + kStake);
  EXPECT_EQ(SUT->getAccount(kDposContract)->balance, dpos_balance_before_confirm - kStake);
}

TEST_F(FinalChainTest, native_dpos_undelegate_v2_cancel_and_claim_all_rewards_same_block_magnolia_active) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kDelegatorStake = 10'000;
  constexpr uint64_t kUndelegationAmount = kDelegatorStake;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 1'000'000'000;
  constexpr uint64_t kGasLimit = 200'000;
  constexpr uint64_t kOwnerInitialBalance = 21'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000'000'000'000'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair& validator = dag_proposer_keys;
  const dev::KeyPair delegator{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};
  const dev::KeyPair receiver{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};
  const auto to_u256 = [](const bytes& value, size_t offset) {
    return u256("0x" + dev::toHex(bytes(value.begin() + offset, value.begin() + offset + 32)));
  };

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_locking_period = 9;
  cfg.genesis.state.hardforks.magnolia_hf.block_num = 1;
  cfg.genesis.state.hardforks.phalaenopsis_hf_block_num = 1;
  cfg.genesis.state.hardforks.cornus_hf.block_num = 1;
  cfg.genesis.state.dpos.delegation_delay = 0;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  validator_info.delegations.emplace(delegator.address(), kDelegatorStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto query_validator = [&](const std::shared_ptr<FinalChain>& chain, const addr_t& who) {
    return chain->call({
        addr_t{},
        kGasPrice,
        kDposContract,
        0,
        0,
        1000000,
        util::EncodingSolidity::packFunctionCall("getValidator(address)", who),
    });
  };

  const auto reward_tx =
      std::make_shared<Transaction>(0, 0, kGasPrice, kGasLimit, dev::bytes(), delegator.secret(), receiver.address(),
                                   cfg.genesis.chain_id);
  const auto reward_result = advance({reward_tx});
  ASSERT_EQ(reward_result->trx_receipts.size(), 1);
  EXPECT_EQ(reward_result->trx_receipts[0].status_code, 1);

  const auto expected_total = SUT->dposTotalAmountDelegated(1);
  const auto pre_claim_dpos_balance = SUT->getAccount(kDposContract)->balance;
  const auto pre_claim_delegator_balance = SUT->getAccount(delegator.address())->balance;

  const auto undelegate_tx = std::make_shared<Transaction>(
      1, 0, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("undelegateV2(address,uint256)", validator.address(),
                                               u256(kUndelegationAmount)),
      delegator.secret(), kDposContract, cfg.genesis.chain_id);
  const auto cancel_tx = std::make_shared<Transaction>(
      2, 0, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("cancelUndelegateV2(address,uint64)", validator.address(), uint64_t{1}),
      delegator.secret(), kDposContract, cfg.genesis.chain_id);
  const auto claim_tx = std::make_shared<Transaction>(
      3, 0, kGasPrice, kGasLimit, util::EncodingSolidity::packFunctionCall("claimAllRewards()"),
      delegator.secret(), kDposContract, cfg.genesis.chain_id);
  const auto lifecycle_result =
      advance({undelegate_tx, cancel_tx, claim_tx}, {.dont_assume_no_logs = true, .dont_assume_all_trx_success = false});
  ASSERT_EQ(lifecycle_result->trx_receipts.size(), 3);
  for (const auto& receipt : lifecycle_result->trx_receipts) {
    EXPECT_EQ(receipt.status_code, 1);
  }
  const auto undelegate_gas = IntrinsicGas(undelegate_tx->getData(), false) + 60'000;
  const auto cancel_gas = IntrinsicGas(cancel_tx->getData(), false) + 60'000;
  const auto claim_gas = IntrinsicGas(claim_tx->getData(), false) + 45'000;
  EXPECT_EQ(lifecycle_result->trx_receipts[0].gas_used, undelegate_gas);
  EXPECT_EQ(lifecycle_result->trx_receipts[0].cumulative_gas_used, undelegate_gas);
  EXPECT_EQ(lifecycle_result->trx_receipts[1].gas_used, cancel_gas);
  EXPECT_EQ(lifecycle_result->trx_receipts[1].cumulative_gas_used, undelegate_gas + cancel_gas);
  EXPECT_EQ(lifecycle_result->trx_receipts[2].gas_used, claim_gas);
  EXPECT_EQ(lifecycle_result->trx_receipts[2].cumulative_gas_used, undelegate_gas + cancel_gas + claim_gas);

  EXPECT_GT(lifecycle_result->trx_receipts[0].logs.size(), 0u);
  const auto& undelegate_log = lifecycle_result->trx_receipts[0].logs.back();
  EXPECT_EQ(undelegate_log.topics.size(), 4u);
  EXPECT_EQ(undelegate_log.topics[0], dev::sha3(dev::asBytes("UndelegatedV2(address,address,uint64,uint256)")));
  EXPECT_EQ(undelegate_log.topics[1], h256(delegator.address(), h256::AlignRight));
  EXPECT_EQ(undelegate_log.topics[2], h256(validator.address(), h256::AlignRight));
  const auto undelegation_id =
      u256("0x" + dev::toHex(undelegate_log.topics[3].asBytes())).convert_to<uint64_t>();
  EXPECT_EQ(undelegation_id, 1u);

  EXPECT_GT(lifecycle_result->trx_receipts[1].logs.size(), 0u);
  const auto& canceled_log = lifecycle_result->trx_receipts[1].logs.back();
  EXPECT_EQ(canceled_log.topics.size(), 4u);
  EXPECT_EQ(canceled_log.topics[0], dev::sha3(dev::asBytes("UndelegateCanceledV2(address,address,uint64,uint256)")));
  EXPECT_EQ(canceled_log.topics[1], h256(delegator.address(), h256::AlignRight));
  EXPECT_EQ(canceled_log.topics[2], h256(validator.address(), h256::AlignRight));
  EXPECT_EQ(to_u256(canceled_log.data, 0), u256(kUndelegationAmount));

  const auto rewards_claimed_topic = dev::sha3(dev::asBytes("RewardsClaimed(address,address,uint256)"));
  u256 claimed_rewards = 0;
  for (const auto& receipt : lifecycle_result->trx_receipts) {
    for (const auto& log : receipt.logs) {
      if (!log.topics.empty() && log.topics[0] == rewards_claimed_topic) {
        ASSERT_EQ(log.topics.size(), 3u);
        EXPECT_EQ(log.topics[1], h256(delegator.address(), h256::AlignRight));
        EXPECT_EQ(log.topics[2], h256(validator.address(), h256::AlignRight));
        claimed_rewards += to_u256(log.data, 0);
      }
    }
  }

  const auto validator_state_after = query_validator(SUT, validator.address()).code_retval;
  ASSERT_GE(validator_state_after.size(), 192u);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(2), expected_total);
  EXPECT_EQ(to_u256(validator_state_after, 160), u256(0));

  LogBloom expected_bloom;
  for (const auto& receipt : lifecycle_result->trx_receipts) {
    expected_bloom |= receipt.bloom();
  }
  EXPECT_EQ(lifecycle_result->final_chain_blk->log_bloom, expected_bloom);

  const auto block_gas = lifecycle_result->final_chain_blk->gas_used;
  const auto expected_delegator_balance =
      pre_claim_delegator_balance + claimed_rewards - u256(block_gas * kGasPrice);
  const auto expected_contract_balance = pre_claim_dpos_balance + u256(block_gas * kGasPrice) - claimed_rewards;
  EXPECT_EQ(SUT->getAccount(delegator.address())->balance, expected_delegator_balance);
  EXPECT_EQ(SUT->getAccount(kDposContract)->balance, expected_contract_balance);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 2);
  const auto restarted_validator_state = query_validator(SUT, validator.address()).code_retval;
  ASSERT_GE(restarted_validator_state.size(), 192u);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(2), expected_total);
  EXPECT_EQ(to_u256(restarted_validator_state, 160), u256(0));
  EXPECT_EQ(SUT->getAccount(delegator.address())->balance, expected_delegator_balance);
  EXPECT_EQ(SUT->getAccount(kDposContract)->balance, expected_contract_balance);
}

TEST_F(FinalChainTest, native_dpos_claim_all_rewards_gas_uses_live_membership_with_nonzero_delay) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kDelegation = 1'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kDelegationDelay = 2;
  constexpr uint64_t kGasPrice = 0;
  constexpr uint64_t kGasLimit = 200'000;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  constexpr uint64_t kDelegatorInitialBalance = 10'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair delegator{dev::Secret("3333333333333333333333333333333333333333333333333333333333333333")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[delegator.address()] = kDelegatorInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = kDelegationDelay;
  cfg.genesis.state.dpos.yield_percentage = 0;
  cfg.genesis.state.hardforks.magnolia_hf.block_num = 1;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const auto delegate_tx = std::make_shared<Transaction>(
      0, kDelegation, kGasPrice, kGasLimit,
      util::EncodingSolidity::packFunctionCall("delegate(address)", validator.address()), delegator.secret(),
      kDposContract, cfg.genesis.chain_id);
  const auto delegate_result = advance({delegate_tx}, {.dont_assume_no_logs = true});
  ASSERT_EQ(delegate_result->trx_receipts.size(), 1);
  const auto delegate_gas = IntrinsicGas(delegate_tx->getData(), false) + 40'000;
  EXPECT_EQ(delegate_result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(delegate_result->trx_receipts[0].gas_used, delegate_gas);
  EXPECT_EQ(delegate_result->trx_receipts[0].cumulative_gas_used, delegate_gas);
  EXPECT_EQ(SUT->dposTotalAmountDelegated(1), u256(kInitialStake + kDelegation));

  const auto claim_tx = std::make_shared<Transaction>(
      1, 0, kGasPrice, kGasLimit, util::EncodingSolidity::packFunctionCall("claimAllRewards()"), delegator.secret(),
      kDposContract, cfg.genesis.chain_id);
  const auto claim_result = advance({claim_tx});
  ASSERT_EQ(claim_result->trx_receipts.size(), 1);
  const auto claim_gas = IntrinsicGas(claim_tx->getData(), false) + 45'000;
  EXPECT_EQ(claim_result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(claim_result->trx_receipts[0].gas_used, claim_gas);
  EXPECT_EQ(claim_result->trx_receipts[0].cumulative_gas_used, claim_gas);
  EXPECT_TRUE(claim_result->trx_receipts[0].logs.empty());
  EXPECT_EQ(claim_result->trx_receipts[0].bloom(), LogBloom());
  EXPECT_EQ(claim_result->final_chain_blk->gas_used, claim_gas);
  EXPECT_EQ(claim_result->final_chain_blk->log_bloom, LogBloom());

  TransactionReceipt expected_claim_receipt;
  expected_claim_receipt.status_code = 1;
  expected_claim_receipt.gas_used = claim_gas;
  expected_claim_receipt.cumulative_gas_used = claim_gas;
  EXPECT_EQ(util::rlp_enc(claim_result->trx_receipts[0]), util::rlp_enc(expected_claim_receipt));

  const auto assert_live_membership_and_delayed_eligibility = [&](const std::shared_ptr<FinalChain>& chain) {
    EXPECT_EQ(chain->dposTotalAmountDelegated(2), u256(kInitialStake + kDelegation));
    EXPECT_EQ(chain->dposEligibleVoteCount(2, validator.address()), kInitialStake / kVoteStep);
    const auto delegator_account = chain->getAccount(delegator.address());
    ASSERT_TRUE(delegator_account);
    EXPECT_EQ(delegator_account->nonce, 2);
    EXPECT_EQ(delegator_account->balance, u256(kDelegatorInitialBalance - kDelegation));
    const auto dpos_account = chain->getAccount(kDposContract);
    ASSERT_TRUE(dpos_account);
    EXPECT_EQ(dpos_account->balance, u256(kInitialStake + kDelegation));
    const auto persisted_claim_receipt = chain->transactionReceipt(2, 0);
    ASSERT_TRUE(persisted_claim_receipt);
    EXPECT_EQ(util::rlp_enc(*persisted_claim_receipt), util::rlp_enc(expected_claim_receipt));
  };

  assert_live_membership_and_delayed_eligibility(SUT);
  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 2);
  assert_live_membership_and_delayed_eligibility(SUT);
}

TEST_F(FinalChainTest, native_dpos_abi_decode_failure_and_invalid_metadata_rollback) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kEligibilityThreshold = 1'000;
  constexpr uint64_t kVoteStep = 1'000;
  constexpr uint64_t kMaximumStake = 30'000;
  constexpr uint64_t kMinimumDeposit = 1'000;
  constexpr uint64_t kGasPrice = 7;
  constexpr uint64_t kGasLimit = 300'000;
  constexpr uint64_t kContinuationGas = 21'000;
  constexpr uint64_t kTruncatedDelegateAmount = 2'000;
  constexpr uint64_t kOwnerInitialBalance = 11'000;
  constexpr uint64_t kSenderInitialBalance = 20'000'000;
  constexpr uint64_t kFixClaimAllBlockNum = 2;

  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");
  const dev::KeyPair owner{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair sender{dev::Secret("3333331111111111111111111111111111111111111111111111111111111111")};
  const dev::KeyPair validator{dev::Secret("2222222222222222222222222222222222222222222222222222222222222222")};
  const dev::KeyPair pending_validator{dev::Secret("4444444444444444444444444444444444444444444444444444444444444444")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kOwnerInitialBalance;
  cfg.genesis.state.initial_balances[sender.address()] = kSenderInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = kEligibilityThreshold;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = kVoteStep;
  cfg.genesis.state.dpos.validator_maximum_stake = kMaximumStake;
  cfg.genesis.state.dpos.minimum_deposit = kMinimumDeposit;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;
  cfg.genesis.state.hardforks.fix_claim_all_block_num = kFixClaimAllBlockNum;
  cfg.genesis.state.hardforks.cornus_hf.block_num = 1;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{validator.address(), owner.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};
  init();
  assume_only_toplevel_transfers = false;

  const auto initial_dpos_account = SUT->getAccount(kDposContract);
  ASSERT_TRUE(initial_dpos_account);
  const auto initial_dpos_balance = initial_dpos_account->balance;
  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);
  ASSERT_EQ(initial_stakes.size(), 1u);
  const auto initial_total_delegated = SUT->dposTotalAmountDelegated(0);
  const auto initial_eligible_votes = SUT->dposEligibleVoteCount(0, validator.address());
  const auto initial_total_votes = SUT->dposEligibleTotalVoteCount(0);

  const auto get_validator = [&](const std::shared_ptr<FinalChain>& chain, const addr_t& who) {
    const auto response = chain->call({
        addr_t{},
        0,
        kDposContract,
        0,
        0,
        1000000,
        // getValidator(address)
        dev::fromHex("0x1904bb2e000000000000000000000000" + who.toString()),
    });
    return response.code_retval;
  };
  const auto get_validator_at_head = [&](const std::shared_ptr<FinalChain>& chain, const addr_t& who) {
    const state_api::EVMTransaction transaction{
        addr_t{}, 0, kDposContract, 0, 0, 1000000, dev::fromHex("0x1904bb2e000000000000000000000000" + who.toString()),
    };
    return chain->call(transaction, chain->lastBlockNumber()).code_retval;
  };

  const auto malformed_delegate_tx = std::make_shared<Transaction>(
      0, 0, kGasPrice, kGasLimit, dev::fromHex("5c19a95c"), sender.secret(), kDposContract, cfg.genesis.chain_id);

  bytes short_claim_all_batch_calldata = dev::fromHex("09b72e00");
  short_claim_all_batch_calldata.resize(35, 0);
  const auto malformed_claim_all_batch_tx = std::make_shared<Transaction>(
      1, 0, kGasPrice, kGasLimit, short_claim_all_batch_calldata, sender.secret(), kDposContract, cfg.genesis.chain_id);

  bytes malformed_batch_high_word = dev::fromHex("09b72e00");
  malformed_batch_high_word.resize(4 + 32, 0);
  malformed_batch_high_word[4] = 0x01;
  malformed_batch_high_word.back() = 0x01;
  malformed_batch_high_word.push_back(0xa5);
  const auto malformed_claim_all_batch_highword_tx = std::make_shared<Transaction>(
      2, 0, kGasPrice, kGasLimit, malformed_batch_high_word, sender.secret(), kDposContract, cfg.genesis.chain_id);

  const auto set_info_calldata =
      util::EncodingSolidity::packFunctionCall("setValidatorInfo(address,string,string)", pending_validator.address(),
                                               dev::asBytes("desc"), dev::asBytes("endpoint"));
  auto malformed_set_info_calldata = bytes(set_info_calldata.begin(), set_info_calldata.begin() + 4 + 2 * 32);
  const auto malformed_set_info_tx = std::make_shared<Transaction>(
      3, 0, kGasPrice, kGasLimit, malformed_set_info_calldata, sender.secret(), kDposContract, cfg.genesis.chain_id);

  const auto short_selector_tx = std::make_shared<Transaction>(4, 0, kGasPrice, kGasLimit, bytes{0x12}, sender.secret(),
                                                               kDposContract, cfg.genesis.chain_id);

  const auto unknown_selector_tx = std::make_shared<Transaction>(5, 0, kGasPrice, kGasLimit, dev::fromHex("deadbeef"),
                                                                 sender.secret(), kDposContract, cfg.genesis.chain_id);

  const auto nonpayable_claim_rewards_tx = std::make_shared<Transaction>(
      6, 1, kGasPrice, kGasLimit,
      dev::fromHex("ef5cfb8c0000000000000000000000000000000000000000000000000000000000000001"), sender.secret(),
      kDposContract, cfg.genesis.chain_id);

  const auto continuation_tx = std::make_shared<Transaction>(7, 0, kGasPrice, kContinuationGas, dev::bytes(),
                                                             sender.secret(), sender.address(), cfg.genesis.chain_id);

  const auto block1_result = advance(
      {malformed_delegate_tx, malformed_claim_all_batch_tx, malformed_claim_all_batch_highword_tx,
       malformed_set_info_tx, short_selector_tx, unknown_selector_tx, nonpayable_claim_rewards_tx, continuation_tx},
      {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true});
  ASSERT_EQ(block1_result->trx_receipts.size(), 8);
  EXPECT_EQ(block1_result->final_chain_blk->number, 1);
  const std::array<uint64_t, 8> block1_expected_gas = {
      IntrinsicGas(malformed_delegate_tx->getData(), false) + 40'000,
      IntrinsicGas(malformed_claim_all_batch_tx->getData(), false),
      IntrinsicGas(malformed_batch_high_word, false),
      IntrinsicGas(malformed_set_info_tx->getData(), false) + 20'000,
      IntrinsicGas(short_selector_tx->getData(), false),
      IntrinsicGas(unknown_selector_tx->getData(), false),
      IntrinsicGas(nonpayable_claim_rewards_tx->getData(), false),
      IntrinsicGas(continuation_tx->getData(), false),
  };
  const std::array<uint8_t, 8> block1_expected_status = {0, 1, 1, 0, 0, 0, 0, 1};
  uint64_t block1_gas_used = 0;
  for (uint64_t idx = 0; idx < block1_result->trx_receipts.size(); ++idx) {
    EXPECT_EQ(block1_result->trx_receipts[idx].status_code, block1_expected_status[idx]);
    EXPECT_EQ(block1_result->trx_receipts[idx].gas_used, block1_expected_gas[idx]);
    EXPECT_EQ(block1_result->trx_receipts[idx].cumulative_gas_used, block1_gas_used + block1_expected_gas[idx]);
    EXPECT_EQ(block1_result->trx_receipts[idx].logs.size(), 0);
    EXPECT_EQ(block1_result->trx_receipts[idx].bloom(), LogBloom());
    block1_gas_used += block1_expected_gas[idx];
  }
  EXPECT_EQ(block1_result->trx_receipts[7].status_code, 1);
  EXPECT_EQ(block1_result->trx_receipts[7].logs.size(), 0);
  EXPECT_EQ(block1_result->trx_receipts[7].bloom(), LogBloom());
  EXPECT_EQ(block1_result->final_chain_blk->log_bloom, LogBloom());

  const u256 block1_gas_cost = u256(block1_gas_used) * kGasPrice;

  const auto assert_rolled_state_after_block1 = [&](const std::shared_ptr<FinalChain>& chain,
                                                    const u256& expected_sender_balance,
                                                    const uint64_t expected_sender_nonce) {
    const auto sender_account = chain->getAccount(sender.address());
    ASSERT_TRUE(sender_account);
    EXPECT_EQ(sender_account->nonce, expected_sender_nonce);
    EXPECT_EQ(sender_account->balance, expected_sender_balance);
    const auto rolled_stakes = chain->dposValidatorsTotalStakes(1);
    ASSERT_EQ(rolled_stakes.size(), 1u);
    EXPECT_EQ(rolled_stakes[0].addr, initial_stakes[0].addr);
    EXPECT_EQ(rolled_stakes[0].stake, initial_stakes[0].stake);
    EXPECT_EQ(chain->dposTotalAmountDelegated(1), initial_total_delegated);
    EXPECT_EQ(chain->dposEligibleVoteCount(1, validator.address()), initial_eligible_votes);
    EXPECT_EQ(chain->dposEligibleTotalVoteCount(1), initial_total_votes);
    EXPECT_EQ(chain->getAccount(kDposContract)->balance, initial_dpos_balance);
  };

  EXPECT_EQ(block1_result->final_chain_blk->gas_used, block1_gas_used);
  EXPECT_EQ(block1_gas_used, block1_result->trx_receipts.back().cumulative_gas_used);
  assert_rolled_state_after_block1(SUT, u256(kSenderInitialBalance) - block1_gas_cost, 8);

  const auto assert_rolled_state_after_block2 = [&](const std::shared_ptr<FinalChain>& chain,
                                                    const u256& expected_sender_balance, const u256& expected_stake,
                                                    const uint64_t expected_nonce) {
    const auto sender_account = chain->getAccount(sender.address());
    ASSERT_TRUE(sender_account);
    EXPECT_EQ(sender_account->nonce, expected_nonce);
    EXPECT_EQ(sender_account->balance, expected_sender_balance);
    const auto stakes = chain->dposValidatorsTotalStakes(2);
    ASSERT_EQ(stakes.size(), 1u);
    EXPECT_EQ(stakes[0].addr, validator.address());
    EXPECT_EQ(stakes[0].stake, expected_stake);
    EXPECT_EQ(chain->dposTotalAmountDelegated(2), initial_total_delegated + expected_stake - initial_stakes[0].stake);
    EXPECT_EQ(chain->dposEligibleVoteCount(2, validator.address()),
              initial_eligible_votes + (kTruncatedDelegateAmount / kVoteStep));
    EXPECT_EQ(chain->dposEligibleTotalVoteCount(2), initial_total_votes + (kTruncatedDelegateAmount / kVoteStep));
  };

  bytes truncated_delegate_calldata(4 + 32, 0);
  const auto truncated_delegate_selector = dev::fromHex("5c19a95c");
  for (size_t i = 0; i < truncated_delegate_selector.size(); ++i) {
    truncated_delegate_calldata[i] = truncated_delegate_selector[i];
  }
  for (size_t i = 0; i < 12; ++i) {
    truncated_delegate_calldata[4 + i] = 0xff;
  }
  const auto truncated_validator_tail = dev::fromHex(validator.address().toString());
  for (size_t i = 0; i < truncated_validator_tail.size(); ++i) {
    truncated_delegate_calldata[16 + i] = truncated_validator_tail[i];
  }
  auto claim_all_batch_tx = std::make_shared<Transaction>(
      9, 0, kGasPrice, kGasLimit,
      dev::fromHex("09b72e0000000000000000000000000000000000000000000000000000000000000001"), sender.secret(),
      kDposContract, cfg.genesis.chain_id);
  const auto truncated_delegate_tx =
      std::make_shared<Transaction>(8, kTruncatedDelegateAmount, kGasPrice, kGasLimit, truncated_delegate_calldata,
                                    sender.secret(), kDposContract, cfg.genesis.chain_id);
  const auto claim_all_trailing_tx = std::make_shared<Transaction>(
      10, 0, kGasPrice, kGasLimit, dev::fromHex("0b83a727a5"), sender.secret(), kDposContract, cfg.genesis.chain_id);

  const auto block2_result = advance({truncated_delegate_tx, claim_all_batch_tx, claim_all_trailing_tx},
                                     {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true});
  ASSERT_EQ(block2_result->trx_receipts.size(), 3);
  EXPECT_EQ(block2_result->final_chain_blk->number, 2);
  EXPECT_EQ(block2_result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(block2_result->trx_receipts[1].status_code, 0);
  EXPECT_EQ(block2_result->trx_receipts[2].status_code, 1);
  EXPECT_EQ(block2_result->trx_receipts[0].logs.size(), 1);
  EXPECT_EQ(block2_result->trx_receipts[1].logs.size(), 0);
  EXPECT_EQ(block2_result->trx_receipts[1].bloom(), LogBloom());
  EXPECT_EQ(block2_result->final_chain_blk->log_bloom, block2_result->trx_receipts[0].bloom());

  const uint64_t block2_expected_gas0 = IntrinsicGas(truncated_delegate_tx->getData(), false) + 40'000;
  const uint64_t block2_expected_gas1 = IntrinsicGas(claim_all_batch_tx->getData(), false);
  const uint64_t block2_expected_gas2 = IntrinsicGas(claim_all_trailing_tx->getData(), false) + 45'000;
  const uint64_t block2_gas_used = block2_expected_gas0 + block2_expected_gas1 + block2_expected_gas2;
  EXPECT_EQ(block2_result->trx_receipts[0].gas_used, block2_expected_gas0);
  EXPECT_EQ(block2_result->trx_receipts[1].gas_used, block2_expected_gas1);
  EXPECT_EQ(block2_result->trx_receipts[2].gas_used, block2_expected_gas2);
  EXPECT_EQ(block2_result->trx_receipts[2].cumulative_gas_used, block2_gas_used);
  const u256 block2_gas_cost = u256(block2_gas_used) * kGasPrice;
  EXPECT_EQ(block2_result->final_chain_blk->gas_used, block2_gas_used);

  EXPECT_GE(block2_result->trx_receipts[0].cumulative_gas_used, block2_result->trx_receipts[0].gas_used);
  EXPECT_EQ(block2_result->trx_receipts[1].cumulative_gas_used, block2_expected_gas0 + block2_expected_gas1);

  const auto sender_balance_after_block1 = u256(kSenderInitialBalance) - block1_gas_cost;
  const auto sender_balance_after_block2 = sender_balance_after_block1 - block2_gas_cost - kTruncatedDelegateAmount;

  EXPECT_EQ(block2_result->final_chain_blk->number, 2);
  const auto expected_stake_after_block2 = u256(initial_stakes[0].stake + kTruncatedDelegateAmount);
  assert_rolled_state_after_block2(SUT, sender_balance_after_block2, expected_stake_after_block2, 11);
  auto proof = dev::sign(pending_validator.secret(), dev::sha3(pending_validator.address())).asBytes();
  proof[64] += 27;
  const auto pending_vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  const bytes pending_vrf_key(pending_vrf_public_key.begin(), pending_vrf_public_key.end());
  const std::string invalid_utf8_description(1, static_cast<char>(0x80));
  const auto invalid_metadata_calldata = util::EncodingSolidity::packFunctionCall(
      "registerValidator(address,bytes,bytes,uint16,string,string)", pending_validator.address(), proof,
      pending_vrf_key, uint16_t{1'000}, dev::asBytes(invalid_utf8_description), dev::asBytes("endpoint"));
  const auto invalid_metadata_tx =
      std::make_shared<Transaction>(11, kMinimumDeposit, kGasPrice, kGasLimit, invalid_metadata_calldata,
                                    sender.secret(), kDposContract, cfg.genesis.chain_id);
  const auto block3_result =
      advance({invalid_metadata_tx}, {.dont_assume_no_logs = true, .dont_assume_all_trx_success = true});
  ASSERT_EQ(block3_result->trx_receipts.size(), 1);
  EXPECT_EQ(block3_result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(block3_result->trx_receipts[0].gas_used, IntrinsicGas(invalid_metadata_tx->getData(), false) + 80'000);
  EXPECT_EQ(block3_result->trx_receipts[0].logs.size(), 2);
  const auto block3_stakes = SUT->dposValidatorsTotalStakes(3);
  ASSERT_EQ(block3_stakes.size(), 2u);
  EXPECT_EQ(block3_stakes[1].addr, pending_validator.address());
  const auto pending_validator_state = get_validator_at_head(SUT, pending_validator.address());
  ASSERT_EQ(pending_validator_state.size(), 416u);
  const auto expect_abi_word = [&](size_t offset, uint64_t value) {
    bytes expected(32, 0);
    for (size_t i = 0; value != 0; ++i, value >>= 8) {
      expected[31 - i] = static_cast<uint8_t>(value);
    }
    EXPECT_EQ(bytes(pending_validator_state.begin() + offset, pending_validator_state.begin() + offset + 32), expected);
  };
  expect_abi_word(0, 32);     // Outer tuple offset.
  expect_abi_word(224, 256);  // Description offset within the tuple.
  expect_abi_word(256, 320);  // Endpoint offset within the tuple.
  expect_abi_word(288, 1);    // Description byte length.
  EXPECT_EQ(pending_validator_state[320], uint8_t{0x80});
  EXPECT_TRUE(std::all_of(pending_validator_state.begin() + 321, pending_validator_state.begin() + 352,
                          [](uint8_t byte) { return byte == 0; }));
  expect_abi_word(352, 8);  // Endpoint byte length.
  EXPECT_EQ(bytes(pending_validator_state.begin() + 384, pending_validator_state.begin() + 392),
            dev::fromHex("656e64706f696e74"));
  EXPECT_TRUE(std::all_of(pending_validator_state.begin() + 392, pending_validator_state.end(),
                          [](uint8_t byte) { return byte == 0; }));

  const auto sender_balance_after_block3 =
      sender_balance_after_block2 - block3_result->trx_receipts[0].gas_used * kGasPrice - kMinimumDeposit;
  const auto restart_validator_state = get_validator(SUT, validator.address());
  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 3);
  const auto restarted_sender = SUT->getAccount(sender.address());
  ASSERT_TRUE(restarted_sender);
  EXPECT_EQ(restarted_sender->nonce, 12);
  EXPECT_EQ(restarted_sender->balance, sender_balance_after_block3);
  EXPECT_EQ(restart_validator_state, get_validator(SUT, validator.address()));
  EXPECT_EQ(pending_validator_state, get_validator_at_head(SUT, pending_validator.address()));
}

TEST_F(FinalChainTest, native_dpos_transfer_into_contract_selector_phalaenopsis_transition) {
  constexpr uint64_t kInitialBalance = 10'000'000;
  constexpr uint64_t kInitialStake = 1'000;
  constexpr uint64_t kGasPrice = 1;
  constexpr uint64_t kGasLimit = 200'000;
  constexpr uint64_t kTransferAmount = 2'000;
  constexpr uint64_t kTransferActionGas = 1'000;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");

  const dev::KeyPair sender{dev::Secret("1111111111111111111111111111111111111111111111111111111111111111")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[sender.address()] = kInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = 100;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = 10;
  cfg.genesis.state.dpos.validator_maximum_stake = 10'000;
  cfg.genesis.state.dpos.minimum_deposit = 1'000;
  cfg.genesis.state.dpos.delegation_locking_period = 0;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;
  cfg.genesis.state.hardforks.magnolia_hf.block_num = 1;
  cfg.genesis.state.hardforks.phalaenopsis_hf_block_num = 2;
  cfg.genesis.state.hardforks.cornus_hf.block_num = 3;

  const auto vrf_public_key = vrf_wrapper::getVrfKeyPair().first;
  state_api::ValidatorInfo validator_info{sender.address(), sender.address(), vrf_public_key, 0, "", "", {}};
  validator_info.delegations.emplace(sender.address(), kInitialStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};

  init();
  assume_only_toplevel_transfers = false;

  const bytes transfer_selector = dev::fromHex("44df8e70");
  bytes transfer_selector_with_trailing = transfer_selector;
  transfer_selector_with_trailing.push_back(0xaa);

  const auto initial_sender_balance = SUT->getAccount(sender.address())->balance;
  const auto initial_dpos_balance = SUT->getAccount(kDposContract)->balance;
  const auto initial_stakes = SUT->dposValidatorsTotalStakes(0);
  const auto initial_total_delegated = SUT->dposTotalAmountDelegated(0);
  const auto initial_validator_votes = SUT->dposEligibleVoteCount(0, sender.address());
  const auto initial_total_votes = SUT->dposEligibleTotalVoteCount(0);

  const auto assert_no_dpos_mutation = [&](const std::shared_ptr<FinalChain>& chain, uint64_t block_number) {
    EXPECT_EQ(chain->dposTotalAmountDelegated(block_number), initial_total_delegated);
    EXPECT_EQ(chain->dposEligibleVoteCount(block_number, sender.address()), initial_validator_votes);
    EXPECT_EQ(chain->dposEligibleTotalVoteCount(block_number), initial_total_votes);
    const auto stakes = chain->dposValidatorsTotalStakes(block_number);
    ASSERT_EQ(stakes.size(), initial_stakes.size());
    for (size_t i = 0; i < stakes.size(); ++i) {
      EXPECT_EQ(stakes[i].addr, initial_stakes[i].addr);
      EXPECT_EQ(stakes[i].stake, initial_stakes[i].stake);
    }
  };

  const auto pre_fork_tx = std::make_shared<Transaction>(
      0, kTransferAmount, kGasPrice, kGasLimit, transfer_selector, sender.secret(), kDposContract,
      cfg.genesis.chain_id);
  const auto pre_fork_trailing_tx = std::make_shared<Transaction>(
      1, 0, kGasPrice, kGasLimit, transfer_selector_with_trailing, sender.secret(), kDposContract,
      cfg.genesis.chain_id);

  const auto pre_fork_selector_gas = IntrinsicGas(transfer_selector, false);
  const auto pre_fork_trailing_gas = IntrinsicGas(transfer_selector_with_trailing, false);
  const auto phalaenopsis_success_gas = IntrinsicGas(transfer_selector, false) + kTransferActionGas;

  const auto block1 = advance({pre_fork_tx, pre_fork_trailing_tx}, {.dont_assume_all_trx_success = true});
  ASSERT_EQ(block1->trx_receipts.size(), 2);
  EXPECT_EQ(block1->final_chain_blk->number, 1);
  EXPECT_EQ(block1->trx_receipts[0].status_code, 0);
  EXPECT_EQ(block1->trx_receipts[0].gas_used, pre_fork_selector_gas);
  EXPECT_EQ(block1->trx_receipts[1].status_code, 0);
  EXPECT_EQ(block1->trx_receipts[1].gas_used, pre_fork_trailing_gas);
  EXPECT_EQ(block1->trx_receipts[0].cumulative_gas_used, pre_fork_selector_gas);
  EXPECT_EQ(block1->trx_receipts[1].cumulative_gas_used, pre_fork_selector_gas + pre_fork_trailing_gas);
  EXPECT_EQ(block1->final_chain_blk->gas_used, pre_fork_selector_gas + pre_fork_trailing_gas);
  EXPECT_EQ(block1->final_chain_blk->log_bloom, LogBloom());
  for (const auto& receipt : block1->trx_receipts) {
    EXPECT_EQ(receipt.logs.size(), 0u);
    EXPECT_EQ(receipt.bloom(), LogBloom());
  }

  const auto post_block1_sender = SUT->getAccount(sender.address());
  ASSERT_TRUE(post_block1_sender);
  const auto post_block1_dpos = SUT->getAccount(kDposContract);
  ASSERT_TRUE(post_block1_dpos);
  const auto expected_sender_after_block1 =
      initial_sender_balance - u256(pre_fork_selector_gas + pre_fork_trailing_gas) * kGasPrice;
  auto expected_dpos_after_block1 = initial_dpos_balance;
#ifdef RUSTAXA_ENABLE_FINAL_CHAIN
  // Rust finalization materializes transaction-fee custody in the DPoS
  // contract account; legacy C++ keeps that reward accounting internal.
  expected_dpos_after_block1 += u256(pre_fork_selector_gas + pre_fork_trailing_gas) * kGasPrice;
#endif
  EXPECT_EQ(post_block1_sender->nonce, 2);
  EXPECT_EQ(post_block1_sender->balance, expected_sender_after_block1);
  EXPECT_EQ(post_block1_dpos->balance, expected_dpos_after_block1);
  assert_no_dpos_mutation(SUT, 1);

  const auto activation_tx = std::make_shared<Transaction>(
      2, kTransferAmount, kGasPrice, kGasLimit, transfer_selector, sender.secret(), kDposContract,
      cfg.genesis.chain_id);
  const auto block2 = advance({activation_tx});
  ASSERT_EQ(block2->trx_receipts.size(), 1);
  EXPECT_EQ(block2->final_chain_blk->number, 2);
  EXPECT_EQ(block2->trx_receipts[0].status_code, 1);
  EXPECT_EQ(block2->trx_receipts[0].gas_used, phalaenopsis_success_gas);
  EXPECT_EQ(block2->trx_receipts[0].cumulative_gas_used, phalaenopsis_success_gas);
  EXPECT_EQ(block2->final_chain_blk->gas_used, phalaenopsis_success_gas);
  EXPECT_EQ(block2->trx_receipts[0].logs.size(), 0u);
  EXPECT_EQ(block2->trx_receipts[0].bloom(), LogBloom());
  EXPECT_EQ(block2->final_chain_blk->log_bloom, LogBloom());

  const auto post_block2_sender = SUT->getAccount(sender.address());
  ASSERT_TRUE(post_block2_sender);
  const auto post_block2_dpos = SUT->getAccount(kDposContract);
  ASSERT_TRUE(post_block2_dpos);
  const auto expected_sender_after_block2 =
      expected_sender_after_block1 - u256(phalaenopsis_success_gas) * kGasPrice - kTransferAmount;
  auto expected_dpos_after_block2 = expected_dpos_after_block1 + kTransferAmount;
#ifdef RUSTAXA_ENABLE_FINAL_CHAIN
  expected_dpos_after_block2 += u256(phalaenopsis_success_gas) * kGasPrice;
#endif
  EXPECT_EQ(post_block2_sender->nonce, 3);
  EXPECT_EQ(post_block2_sender->balance, expected_sender_after_block2);
  EXPECT_EQ(post_block2_dpos->balance, expected_dpos_after_block2);
  assert_no_dpos_mutation(SUT, 2);

  const auto cornus_block_tx = std::make_shared<Transaction>(
      3, kTransferAmount, kGasPrice, kGasLimit, transfer_selector, sender.secret(), kDposContract,
      cfg.genesis.chain_id);
  const auto block3 = advance({cornus_block_tx});
  ASSERT_EQ(block3->trx_receipts.size(), 1);
  EXPECT_EQ(block3->final_chain_blk->number, 3);
  EXPECT_EQ(block3->trx_receipts[0].status_code, 1);
  EXPECT_EQ(block3->trx_receipts[0].gas_used, phalaenopsis_success_gas);
  EXPECT_EQ(block3->trx_receipts[0].cumulative_gas_used, phalaenopsis_success_gas);
  EXPECT_EQ(block3->final_chain_blk->gas_used, phalaenopsis_success_gas);
  EXPECT_EQ(block3->trx_receipts[0].logs.size(), 0u);
  EXPECT_EQ(block3->trx_receipts[0].bloom(), LogBloom());
  EXPECT_EQ(block3->final_chain_blk->log_bloom, LogBloom());

  const auto post_block3_sender = SUT->getAccount(sender.address());
  ASSERT_TRUE(post_block3_sender);
  const auto post_block3_dpos = SUT->getAccount(kDposContract);
  ASSERT_TRUE(post_block3_dpos);
  const auto expected_sender_after_block3 =
      expected_sender_after_block2 - u256(phalaenopsis_success_gas) * kGasPrice - kTransferAmount;
  auto expected_dpos_after_block3 = expected_dpos_after_block2 + kTransferAmount;
#ifdef RUSTAXA_ENABLE_FINAL_CHAIN
  expected_dpos_after_block3 += u256(phalaenopsis_success_gas) * kGasPrice;
#endif
  EXPECT_EQ(post_block3_sender->nonce, 4);
  EXPECT_EQ(post_block3_sender->balance, expected_sender_after_block3);
  EXPECT_EQ(post_block3_dpos->balance, expected_dpos_after_block3);
  assert_no_dpos_mutation(SUT, 3);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 3);
  const auto restarted_sender = SUT->getAccount(sender.address());
  ASSERT_TRUE(restarted_sender);
  EXPECT_EQ(restarted_sender->nonce, 4);
  EXPECT_EQ(restarted_sender->balance, expected_sender_after_block3);
  const auto restarted_dpos_contract = SUT->getAccount(kDposContract);
  ASSERT_TRUE(restarted_dpos_contract);
  EXPECT_EQ(restarted_dpos_contract->balance, expected_dpos_after_block3);
  assert_no_dpos_mutation(SUT, 3);
}

TEST_F(FinalChainTest, native_dpos_pre_cornus_nonpayable_value_transfers_on_success) {
  constexpr uint64_t kInitialStake = 10'000;
  constexpr uint64_t kInitialBalance = 1'000'000;
  constexpr uint64_t kGasPrice = 1;
  constexpr uint64_t kTransferredValue = 1;
  const addr_t kDposContract("0x00000000000000000000000000000000000000FE");
  const dev::KeyPair owner{dev::Secret("5555555555555555555555555555555555555555555555555555555555555555")};
  const dev::KeyPair validator{dev::Secret("6666666666666666666666666666666666666666666666666666666666666666")};

  cfg.genesis.state.initial_balances.clear();
  cfg.genesis.state.initial_balances[owner.address()] = kInitialBalance;
  cfg.genesis.state.dpos.eligibility_balance_threshold = 1'000;
  cfg.genesis.state.dpos.vote_eligibility_balance_step = 1'000;
  cfg.genesis.state.dpos.validator_maximum_stake = 30'000;
  cfg.genesis.state.dpos.minimum_deposit = 1'000;
  cfg.genesis.state.dpos.delegation_delay = 0;
  cfg.genesis.state.dpos.yield_percentage = 0;
  cfg.genesis.state.hardforks.cornus_hf.block_num = 2;

  state_api::ValidatorInfo validator_info{
      validator.address(), owner.address(), vrf_wrapper::getVrfKeyPair().first, 0, "old", "old-endpoint", {}};
  validator_info.delegations.emplace(owner.address(), kInitialStake);
  cfg.genesis.state.dpos.initial_validators = {validator_info};
  init();
  assume_only_toplevel_transfers = false;

  const auto initial_owner_balance = SUT->getAccount(owner.address())->balance;
  const auto initial_dpos_balance = SUT->getAccount(kDposContract)->balance;
  const auto calldata =
      util::EncodingSolidity::packFunctionCall("setValidatorInfo(address,string,string)", validator.address(),
                                               dev::asBytes("new"), dev::asBytes("new-endpoint"));
  const auto transaction = std::make_shared<Transaction>(0, kTransferredValue, kGasPrice, 100'000, calldata,
                                                         owner.secret(), kDposContract, cfg.genesis.chain_id);
  const auto result = advance({transaction}, {.dont_assume_no_logs = true});
  ASSERT_EQ(result->trx_receipts.size(), 1);
  const auto expected_gas = IntrinsicGas(calldata, false) + 20'000;
  EXPECT_EQ(result->trx_receipts[0].status_code, 1);
  EXPECT_EQ(result->trx_receipts[0].gas_used, expected_gas);
  EXPECT_EQ(result->trx_receipts[0].logs.size(), 1);
  const auto owner_account = SUT->getAccount(owner.address());
  ASSERT_TRUE(owner_account);
  EXPECT_EQ(owner_account->nonce, 1);
  EXPECT_EQ(owner_account->balance, initial_owner_balance - expected_gas - kTransferredValue);
  EXPECT_EQ(SUT->getAccount(kDposContract)->balance, initial_dpos_balance + kTransferredValue);

  SUT.reset();
  SUT = std::make_shared<final_chain::FinalChain>(db, cfg, addr_t{});
  EXPECT_EQ(SUT->lastBlockNumber(), 1);
  EXPECT_EQ(SUT->getAccount(owner.address())->balance, initial_owner_balance - expected_gas - kTransferredValue);
  EXPECT_EQ(SUT->getAccount(kDposContract)->balance, initial_dpos_balance + kTransferredValue);
}

// This test should be last as state_api isn't destructed correctly because of exception
TEST_F(FinalChainTest, initial_validator_exceed_maximum_stake) {
  const dev::KeyPair key = dev::KeyPair::create();
  const dev::KeyPair validator_key = dev::KeyPair::create();
  const auto vrf_pub_key = taraxa::vrf_wrapper::getVrfKeyPair().first;
  fillConfigForGenesisTests(key.address());

  state_api::ValidatorInfo validator{validator_key.address(), key.address(), vrf_pub_key, 0, "", "", {}};
  validator.delegations.emplace(key.address(), cfg.genesis.state.dpos.validator_maximum_stake);
  validator.delegations.emplace(validator_key.address(), cfg.genesis.state.dpos.minimum_deposit);
  cfg.genesis.state.dpos.initial_validators.emplace_back(validator);

  EXPECT_THROW(init(), std::exception);
}

}  // namespace taraxa::final_chain

TARAXA_TEST_MAIN({})
