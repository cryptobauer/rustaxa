#pragma once

#include "dag/dag.hpp"
#include "final_chain/final_chain.hpp"
#include "mutation.hpp"
#include "network/http_server.hpp"
#include "network/live_status.hpp"
#include "network/network.hpp"
#include "query.hpp"
#include "subscription.hpp"
namespace taraxa::net {

// GraphQlOperations is the external HTTP processor's minimal GraphQL boundary.
// Callers provide already-wired operation roots, keeping app consensus managers
// out of the primary HTTP processing API.
struct GraphQlOperations {
  std::shared_ptr<graphql::taraxa::Query> query;
  std::shared_ptr<graphql::taraxa::Mutation> mutation;
  std::shared_ptr<graphql::taraxa::Subscription> subscription;
};

class GraphQlHttpProcessor final : public HttpProcessor {
 public:
  explicit GraphQlHttpProcessor(GraphQlOperations operations);
  GraphQlHttpProcessor(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                       std::shared_ptr<::taraxa::DagManager> dag_manager,
#ifndef RUSTAXA_ENABLE
                       std::shared_ptr<::taraxa::PbftManager> pbft_manager,
#endif
                       std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                       std::shared_ptr<::taraxa::DbStorage> db,
#ifdef RUSTAXA_ENABLE
                       graphql::taraxa::QueryGasPriceReader gas_price_reader,
#else
                       std::shared_ptr<::taraxa::GasPricer> gas_pricer,
#endif
                       std::weak_ptr<::taraxa::Network> network, uint64_t chain_id,
                       ::taraxa::net::LiveStatusReader live_status = {});
  Response process(const Request& request) override;

 private:
  Response createErrResponse(std::string&& = "");
  Response createErrResponse(graphql::response::Value&& error_value);
  Response createOkResponse(std::string&& response_body);

 private:
  std::shared_ptr<graphql::taraxa::Query> query_;
  std::shared_ptr<graphql::taraxa::Mutation> mutation_;
  std::shared_ptr<graphql::taraxa::Subscription> subscription_;
  graphql::taraxa::Operations operations_;
};

}  // namespace taraxa::net
