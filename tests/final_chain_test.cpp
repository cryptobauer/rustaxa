#include "final_chain/final_chain.hpp"

#include <libdevcore/CommonData.h>

#include <array>
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
