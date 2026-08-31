#include "final_chain/state_api.hpp"

#include <libdevcore/CommonJS.h>

#include <algorithm>
#include <boost/filesystem.hpp>
#include <fstream>
#include <vector>

#include "common/encoding_rlp.hpp"
#ifndef RUSTAXA_ENABLE
#include "pbft/pbft_manager.hpp"
#endif
#include "slashing_manager/slashing_manager.hpp"
#include "test_util/test_util.hpp"
#ifndef RUSTAXA_ENABLE
#include "transaction/gas_pricer.hpp"
#endif
#ifndef RUSTAXA_ENABLE
#include "vote_manager/vote_manager.hpp"
#endif

namespace taraxa::state_api {
using boost::filesystem::create_directories;
using boost::filesystem::path;
using boost::filesystem::remove_all;
using boost::filesystem::temp_directory_path;
using namespace std;
// using namespace core_tests;

struct StateAPITest : NodesTest {};

struct TestBlock {
  h256 hash;
  h256 state_root;
  EVMBlock evm_block;
  vector<EVMTransaction> transactions;

  RLP_FIELDS_DEFINE_INPLACE(hash, state_root, evm_block, transactions)
};

#ifdef RUSTAXA_ENABLE
/** Test-only decoder for the stable concrete-state identity carried inside opaque StateAPI provenance RLP. */
struct TestConcreteStateIdentity {
  uint64_t policy_version = 0;
  h256 database_identity;
  h256 chain_identity;

  RLP_FIELDS_DEFINE_INPLACE(policy_version, database_identity, chain_identity)
};

/** Test-only canonical marker used to exercise the opaque stage/discard boundary. */
struct TestConcreteExecutionMarker {
  TestConcreteStateIdentity identity;
  uint64_t generation = 0;
  h256 plan_hash;
  EthBlockNumber period = 0;
  StateDescriptor prior_state;
  h256 transactions_hash;
  h256 rewards_hash;

  RLP_FIELDS_DEFINE_INPLACE(identity, generation, plan_hash, period, prior_state, transactions_hash, rewards_hash)
};

/** Test-only decoder for the committed fields required to construct the next exact marker. */
struct TestConcreteStateProvenance {
  TestConcreteStateIdentity identity;
  uint64_t generation = 0;
  h256 plan_hash;
  StateDescriptor committed_state;
  h256 transactions_hash;
  h256 rewards_hash;
  h256 projection_hash;
  h256 catalog_hash;

  RLP_FIELDS_DEFINE_INPLACE(identity, generation, plan_hash, committed_state, transactions_hash, rewards_hash,
                            projection_hash, catalog_hash)
};
#endif

template <typename T>
T parse_rlp_file(path const& p) {
  ifstream strm(p.string());
  T ret;
  util::rlp(dev::RLP(string(istreambuf_iterator(strm), {}), 0), ret);
  return ret;
}

TEST_F(StateAPITest, DISABLED_dpos_integration) {
  // Config chain_cfg;

  // DPOSQuery::AccountQuery acc_q;
  // acc_q.with_staking_balance = true;
  // acc_q.with_outbound_deposits = true;
  // acc_q.with_inbound_deposits = true;
  // DPOSQuery q;
  // q.with_eligible_count = true;
  // q.account_queries[make_addr(1)] = acc_q;
  // q.account_queries[make_addr(2)] = acc_q;
  // q.account_queries[make_addr(3)] = acc_q;

  // u256 addr_1_bal_expected = 100000000;
  // chain_cfg.initial_balances[make_addr(1)] = addr_1_bal_expected;
  // auto& dpos_cfg = chain_cfg.dpos.emplace();
  // dpos_cfg.delegation_delay = 2;
  // dpos_cfg.delegation_locking_period = 4;
  // dpos_cfg.eligibility_balance_threshold = 1000;
  // dpos_cfg.vote_eligibility_balance_step = 1000;
  // addr_1_bal_expected -= dpos_cfg.genesis_state[make_addr(1)][make_addr(1)] = dpos_cfg.eligibility_balance_threshold;

  // uint64_t curr_blk = 0;
  // StateAPI SUT([&](auto /*n*/) -> h256 { assert(false); },  //
  //              chain_cfg,
  //              {
  //                  10,
  //                  1,
  //              },
  //              {
  //                  (data_dir / "state").string(),
  //              });

  // unordered_set<addr_t> expected_eligible_set;
  // decltype(DPOSQueryResult().account_results) exp_q_acc_res;
  // auto CHECK = [&] {
  //   for (auto& [addr, res] : exp_q_acc_res) {
  //     res.is_eligible = expected_eligible_set.count(addr);
  //   }
  //   for (auto const& addr : expected_eligible_set) {
  //     exp_q_acc_res[addr].is_eligible = true;
  //   }
  //   string meta = "at block " + to_string(curr_blk);
  //   EXPECT_EQ(addr_1_bal_expected, SUT.getAccount(curr_blk, make_addr(1))->balance) << meta;
  //   for (auto const& addr : expected_eligible_set) {
  //     EXPECT_TRUE(SUT.dposIsEligible(curr_blk, addr)) << meta;
  //     EXPECT_EQ(SUT.dposEligibleVoteCount(curr_blk, addr), 1) << meta;
  //   }
  //   EXPECT_EQ(SUT.dposEligibleTotalVoteCount(curr_blk), expected_eligible_set.size()) << meta;
  //   // auto q_res = SUT.dpos_query(curr_blk, q);
  //   EXPECT_EQ(q_res.eligible_count, expected_eligible_set.size()) << meta;
  //   for (auto& [addr, res_exp] : exp_q_acc_res) {
  //     auto& res_act = q_res.account_results[addr];
  //     auto meta_ = meta + " @ " + addr.hex();
  //     EXPECT_EQ(res_exp.staking_balance, res_act.staking_balance) << meta_;
  //     EXPECT_EQ(res_exp.is_eligible, res_act.is_eligible) << meta_;
  //     for (auto [label, deposits_p_exp, deposits_p_act] : {
  //              tuple{"in", &res_exp.inbound_deposits, &res_act.inbound_deposits},
  //              tuple{"out", &res_exp.outbound_deposits, &res_act.outbound_deposits},
  //          }) {
  //       auto& deposits_exp = *deposits_p_exp;
  //       auto& deposits_act = *deposits_p_act;
  //       auto meta__ = meta_ + " (" + label + ")";
  //       EXPECT_EQ(deposits_exp.size(), deposits_act.size()) << meta__;
  //       for (auto& [addr, deposit_v_exp] : deposits_exp) {
  //         EXPECT_EQ(deposit_v_exp, deposits_act[addr]) << meta__;
  //       }
  //     }
  //   }
  // };
  // auto EXEC_AND_CHECK = [&](vector<EVMTransaction> const& trxs) {
  //   auto result = SUT.transition_state({}, trxs);
  //   SUT.transition_state_commit();
  //   for (auto& r : result.execution_results) {
  //     EXPECT_TRUE(r.code_retval.empty());
  //     EXPECT_TRUE(r.code_err.empty());
  //   }
  //   ++curr_blk;
  //   CHECK();
  // };

  // DPOSTransfers transfers;
  // auto make_dpos_trx = [&] {
  //   StateAPI::DPOSTransactionPrototype trx_proto(transfers);
  //   transfers = {};
  //   EVMTransaction trx;
  //   trx.from = make_addr(1);
  //   trx.to = trx_proto.to;
  //   trx.value = trx_proto.value;
  //   trx.input = trx_proto.input;
  //   trx.gas = trx_proto.minimal_gas;
  //   return trx;
  // };

  // expected_eligible_set = {make_addr(1)};
  // exp_q_acc_res[make_addr(1)].inbound_deposits[make_addr(1)] =
  //     exp_q_acc_res[make_addr(1)].outbound_deposits[make_addr(1)] = exp_q_acc_res[make_addr(1)].staking_balance =
  //     1000;
  // CHECK();

  // addr_1_bal_expected -= transfers[make_addr(2)].value = 1000;
  // addr_1_bal_expected -= transfers[make_addr(3)].value = 999;
  // EXEC_AND_CHECK({make_dpos_trx()});

  // transfers[make_addr(2)] = {1, true};
  // addr_1_bal_expected -= transfers[make_addr(3)].value = 1;
  // EXEC_AND_CHECK({make_dpos_trx()});

  // expected_eligible_set = {make_addr(1), make_addr(2)};
  // exp_q_acc_res[make_addr(1)].outbound_deposits[make_addr(2)] =
  //     exp_q_acc_res[make_addr(2)].inbound_deposits[make_addr(1)] = exp_q_acc_res[make_addr(2)].staking_balance =
  //     1000;
  // exp_q_acc_res[make_addr(1)].outbound_deposits[make_addr(3)] =
  //     exp_q_acc_res[make_addr(3)].inbound_deposits[make_addr(1)] = exp_q_acc_res[make_addr(3)].staking_balance = 999;
  // EXEC_AND_CHECK({});

  // expected_eligible_set = {make_addr(1), make_addr(2), make_addr(3)};
  // exp_q_acc_res[make_addr(1)].outbound_deposits[make_addr(3)] =
  //     exp_q_acc_res[make_addr(3)].inbound_deposits[make_addr(1)] = 1000;
  // exp_q_acc_res[make_addr(3)].staking_balance = 1000;
  // EXEC_AND_CHECK({});
  // EXEC_AND_CHECK({});

  // addr_1_bal_expected += 1;
  // expected_eligible_set = {make_addr(1), make_addr(3)};
  // exp_q_acc_res[make_addr(1)].outbound_deposits[make_addr(2)] =
  //     exp_q_acc_res[make_addr(2)].inbound_deposits[make_addr(1)] = exp_q_acc_res[make_addr(2)].staking_balance = 999;
  // EXEC_AND_CHECK({});
  // EXEC_AND_CHECK({});
  // EXEC_AND_CHECK({});
  // EXEC_AND_CHECK({});
}

TEST_F(StateAPITest, DISABLED_eth_mainnet_smoke) {
  auto test_blocks =
      parse_rlp_file<vector<TestBlock>>(path(__FILE__).parent_path().parent_path() / "submodules" / "taraxa-evm" /
                                        "taraxa" / "data" / "eth_mainnet_blocks_0_300000.rlp");

  Config chain_config;
  auto initial_balances_rlp_hex_c = taraxa_evm_mainnet_initial_balances();
  auto initial_balances_rlp =
      dev::jsToBytes(string((char*)initial_balances_rlp_hex_c.Data, initial_balances_rlp_hex_c.Len));
  util::rlp(dev::RLP(initial_balances_rlp), chain_config.initial_balances);

  Opts opts;
  opts.expected_max_trx_per_block = 300;
  opts.max_trie_full_node_levels_to_cache = 4;

  StateAPI SUT([&](auto n) { return test_blocks[n].hash; },  //
               chain_config, opts,
               {
                   (data_dir / "state").string(),
               });

  ASSERT_EQ(test_blocks[0].state_root, SUT.get_last_committed_state_descriptor().state_root);
  size_t num_blk_to_exec = 150000;  // test_blocks.size() will provide more coverage but will be slower
  long double progress_pct = 0, progress_pct_log_threshold = 0;
  auto one_blk_in_pct = (long double)100 / num_blk_to_exec;
  for (size_t blk_num = 1; blk_num < num_blk_to_exec; ++blk_num) {
    if ((progress_pct += one_blk_in_pct) >= progress_pct_log_threshold) {
      cout << "progress: " << uint(progress_pct) << "%" << endl;
      progress_pct_log_threshold += 10;
    }
    auto const& test_block = test_blocks[blk_num];
    SUT.execute_transactions(test_block.evm_block, test_block.transactions);
    const auto& result = SUT.distribute_rewards({});
    ASSERT_EQ(result.state_root, test_block.state_root);
    SUT.transition_state_commit();
  }
}

#ifdef RUSTAXA_ENABLE
TEST_F(StateAPITest, staged_multi_transaction_roots_precede_rewards_and_commit) {
  auto node_configs = make_node_cfgs(1, 1, 5);
  const auto& chain_config = node_configs.front().genesis.state;
  Opts opts;
  opts.expected_max_trx_per_block = 2;
  StateAPI state_api([](EthBlockNumber) { return ZeroHash(); }, chain_config, opts,
                     {(data_dir / "intermediate_root_state").string()});

  TestConcreteStateProvenance activated;
  const auto activated_rlp = state_api.activate_concrete_root_policy(h256::random());
  util::rlp(dev::RLP(activated_rlp), activated);
  TestConcreteExecutionMarker marker{
      .identity = activated.identity,
      .generation = activated.generation + 1,
      .plan_hash = h256::random(),
      .period = activated.committed_state.blk_num + 1,
      .prior_state = activated.committed_state,
      .transactions_hash = h256::random(),
      .rewards_hash = h256::random(),
  };
  const auto marker_rlp = util::rlp_enc(marker);
  state_api.stage_concrete_execution(marker_rlp);

  const auto balances = effective_initial_balances(chain_config);
  ASSERT_FALSE(balances.empty());
  const auto sender = balances.begin()->first;
  const addr_t recipient("00000000000000000000000000000000000000c9");
  const std::vector<EVMTransaction> transactions{
      EVMTransaction{sender, 1, recipient, 0, 1, 21'000, {}},
      EVMTransaction{sender, 1, recipient, 1, 1, 21'000, {}},
  };
  const auto& execution = state_api.execute_transactions(EVMBlock{sender, 1'000'000, marker.period, 1}, transactions);
  ASSERT_EQ(execution.execution_results.size(), transactions.size());
  EXPECT_NE(state_api.post_transaction_state_root(), ZeroHash());
  EXPECT_EQ(state_api.get_last_committed_state_descriptor().state_root, marker.prior_state.state_root);

  const auto& rewards = state_api.distribute_rewards({});
  const auto projection_rlp = state_api.get_concrete_state_projection();
  const dev::RLP projection(projection_rlp);
  ASSERT_EQ(projection[7].itemCount(), transactions.size());
  const auto first_root = projection[7][0][3][1].toHash<h256>();
  const auto second_root = projection[7][1][3][1].toHash<h256>();
  EXPECT_NE(first_root, ZeroHash());
  EXPECT_NE(second_root, first_root);
  EXPECT_EQ(second_root, state_api.post_transaction_state_root());
  EXPECT_EQ(projection[6][1].toHash<h256>(), rewards.state_root);

  const auto rewards_root = rewards.state_root;
  const auto total_reward = rewards.total_reward;
  const auto& retry = state_api.distribute_rewards({});
  EXPECT_EQ(retry.state_root, rewards_root);
  EXPECT_EQ(retry.total_reward, total_reward);
  EXPECT_EQ(state_api.get_concrete_state_projection(), projection_rlp);

  state_api.discard_concrete_execution(marker_rlp);
  EXPECT_EQ(state_api.get_last_committed_state_descriptor().state_root, marker.prior_state.state_root);
}

TEST_F(StateAPITest, concrete_projection_discard_reopens_exact_committed_state) {
  auto node_configs = make_node_cfgs(1, 1, 5);
  const auto& chain_config = node_configs.front().genesis.state;
  Opts opts;
  opts.expected_max_trx_per_block = 1;
  StateAPI state_api([](EthBlockNumber) { return ZeroHash(); }, chain_config, opts,
                     {(data_dir / "concrete_projection_discard_state").string()});

  const auto chain_identity = h256::random();
  const auto activated_rlp = state_api.activate_concrete_root_policy(chain_identity);
  TestConcreteStateProvenance activated;
  util::rlp(dev::RLP(activated_rlp), activated);
  EXPECT_EQ(activated.identity.policy_version, 1);
  EXPECT_EQ(activated.identity.chain_identity, chain_identity);
  EXPECT_NE(activated.catalog_hash, dev::sha3(bytes{0xc0}));
  EXPECT_EQ(state_api.get_concrete_state_provenance(), activated_rlp);
  EXPECT_FALSE(state_api.get_pending_concrete_execution());

  TestConcreteExecutionMarker marker{
      .identity = activated.identity,
      .generation = activated.generation + 1,
      .plan_hash = h256::random(),
      .period = activated.committed_state.blk_num + 1,
      .prior_state = activated.committed_state,
      .transactions_hash = h256::random(),
      .rewards_hash = h256::random(),
  };
  const auto marker_rlp = util::rlp_enc(marker);
  state_api.stage_concrete_execution(marker_rlp);
  ASSERT_TRUE(state_api.get_pending_concrete_execution());
  EXPECT_EQ(*state_api.get_pending_concrete_execution(), marker_rlp);

  state_api.execute_transactions(EVMBlock{dev::ZeroAddress, 1'000'000, 1, 1}, {});
  state_api.distribute_rewards({});
  EXPECT_FALSE(state_api.get_concrete_state_projection().empty());
  state_api.discard_concrete_execution(marker_rlp);
  EXPECT_FALSE(state_api.get_pending_concrete_execution());
  const auto reopened = state_api.get_last_committed_state_descriptor();
  EXPECT_EQ(reopened.blk_num, activated.committed_state.blk_num);
  EXPECT_EQ(reopened.state_root, activated.committed_state.state_root);

  state_api.stage_concrete_execution(marker_rlp);
  state_api.execute_transactions(EVMBlock{dev::ZeroAddress, 1'000'000, 1, 1}, {});
  const auto& rewards = state_api.distribute_rewards({});
  const auto projection_rlp = state_api.get_concrete_state_projection();
  EXPECT_GT(dev::RLP(projection_rlp)[9].itemCount(), 0);
  const auto projection_hash = dev::sha3(projection_rlp);
  const auto catalog_hash = dev::RLP(projection_rlp)[12].toHash<h256>();
  const auto expected_provenance_rlp = util::rlp_enc(TestConcreteStateProvenance{
      .identity = marker.identity,
      .generation = marker.generation,
      .plan_hash = marker.plan_hash,
      .committed_state = StateDescriptor{marker.period, rewards.state_root},
      .transactions_hash = marker.transactions_hash,
      .rewards_hash = marker.rewards_hash,
      .projection_hash = projection_hash,
      .catalog_hash = catalog_hash,
  });
  state_api.concrete_commit(projection_hash, expected_provenance_rlp);

  const auto committed = state_api.get_last_committed_state_descriptor();
  EXPECT_EQ(committed.blk_num, marker.period);
  EXPECT_EQ(committed.state_root, rewards.state_root);
  EXPECT_FALSE(state_api.get_pending_concrete_execution());
  TestConcreteStateProvenance provenance;
  const auto committed_provenance_rlp = state_api.get_concrete_state_provenance();
  util::rlp(dev::RLP(committed_provenance_rlp), provenance);
  EXPECT_EQ(provenance.generation, marker.generation);
  EXPECT_EQ(provenance.plan_hash, marker.plan_hash);
  EXPECT_EQ(provenance.projection_hash, projection_hash);
  EXPECT_THROW(state_api.activate_concrete_root_policy(h256::random()), TaraxaEVMError);
  EXPECT_EQ(state_api.get_last_committed_state_descriptor().state_root, committed.state_root);
}

TEST_F(StateAPITest, concrete_root_policy_persists_nonempty_genesis_catalog_on_restart) {
  auto node_configs = make_node_cfgs(1, 1, 5);
  const auto& chain_config = node_configs.front().genesis.state;
  Opts opts;
  opts.expected_max_trx_per_block = 1;
  const auto state_path = (data_dir / "concrete_genesis_catalog_restart_state").string();
  const auto chain_identity = h256::random();
  h256 genesis_catalog_hash;

  {
    StateAPI state_api([](EthBlockNumber) { return ZeroHash(); }, chain_config, opts, {state_path});
    TestConcreteStateProvenance activated;
    const auto activated_rlp = state_api.activate_concrete_root_policy(chain_identity);
    util::rlp(dev::RLP(activated_rlp), activated);
    genesis_catalog_hash = activated.catalog_hash;
    EXPECT_NE(genesis_catalog_hash, dev::sha3(bytes{0xc0}));
  }

  {
    StateAPI reopened([](EthBlockNumber) { return ZeroHash(); }, chain_config, opts, {state_path});
    TestConcreteStateProvenance activated;
    const auto activated_rlp = reopened.activate_concrete_root_policy(chain_identity);
    util::rlp(dev::RLP(activated_rlp), activated);
    EXPECT_EQ(activated.catalog_hash, genesis_catalog_hash);
  }
}
#endif

#ifndef RUSTAXA_ENABLE
TEST_F(StateAPITest, slashing) {
  auto node_cfgs = make_node_cfgs(1, 1, 5);
  // Option 2: more sophisticated and longer test
  // auto node_cfgs = make_node_cfgs(4, 4, 5);
  for (auto& cfg : node_cfgs) {
    cfg.genesis.state.dpos.delegation_delay = 2;
    cfg.genesis.state.hardforks.magnolia_hf.jail_time = 2;
    cfg.genesis.state.hardforks.magnolia_hf.block_num = 6;
    cfg.report_malicious_behaviour = true;
  }

  auto nodes = launch_nodes(node_cfgs);
  auto node = *nodes.begin();
  auto node_cfg = node_cfgs.begin();
  ASSERT_EQ(true, node->getFinalChain()->dposIsEligible(node->getFinalChain()->lastBlockNumber(), node->getAddress()));

  ASSERT_HAPPENS({10s, 100ms}, [&](auto& ctx) {
    WAIT_EXPECT_GE(ctx, node->getFinalChain()->lastBlockNumber(),
                   node_cfg->genesis.state.hardforks.magnolia_hf.block_num)
  });

  auto slashing_manager = std::make_shared<SlashingManager>(*node_cfg, node->getFinalChain(),
                                                            node->getTransactionManager(), node->getGasPricer());
  auto preactivation_vote_a = node->getVoteManager()->generateVote(blk_hash_t{3}, PbftVoteTypes::cert_vote, 5, 1, 3,
                                                                   node_cfg->getFirstWallet());
  auto preactivation_vote_b = node->getVoteManager()->generateVote(blk_hash_t{4}, PbftVoteTypes::cert_vote, 5, 1, 3,
                                                                   node_cfg->getFirstWallet());
  ASSERT_FALSE(slashing_manager->submitDoubleVotingProof(preactivation_vote_a, preactivation_vote_b));

  auto vote_a = node->getVoteManager()->generateVote(blk_hash_t{1}, PbftVoteTypes::cert_vote, 6, 1, 3,
                                                     node_cfg->getFirstWallet());
  auto vote_b = node->getVoteManager()->generateVote(blk_hash_t{2}, PbftVoteTypes::cert_vote, 6, 1, 3,
                                                     node_cfg->getFirstWallet());
  ASSERT_TRUE(slashing_manager->submitDoubleVotingProof(vote_a, vote_b));

  // After few blocks malicious validator should be jailed
  ASSERT_HAPPENS({10s, 100ms}, [&](auto& ctx) {
    WAIT_EXPECT_EQ(ctx, false,
                   node->getFinalChain()->dposIsEligible(node->getFinalChain()->lastBlockNumber(), node->getAddress()))
  });

  // Option 2: more sophisticated and longer test
  // After few blocks malicious validator should be unjailed
  //  ASSERT_HAPPENS({5s, 100ms}, [&](auto& ctx) {
  //    WAIT_EXPECT_EQ(
  //        ctx, true,
  //        node->getFinalChain()->dposIsEligible(node->getFinalChain()->lastBlockNumber(), node->getAddress()))
  //  });
}
#endif

}  // namespace taraxa::state_api

TARAXA_TEST_MAIN({})
