#include "graphql/http_processor.hpp"

#include <stdexcept>

#include "common/jsoncpp.hpp"
#include "common/util.hpp"
#include "graphqlservice/GraphQLService.h"
#include "graphqlservice/JSONResponse.h"

namespace taraxa::net {

using namespace graphql;

namespace {
std::shared_ptr<graphql::taraxa::Query> requireGraphQlQuery(std::shared_ptr<graphql::taraxa::Query> query) {
  if (!query) {
    throw std::invalid_argument("GRAPHQL_HTTP_PROCESSOR_MISSING_QUERY");
  }
  return query;
}

std::shared_ptr<graphql::taraxa::Mutation> requireGraphQlMutation(std::shared_ptr<graphql::taraxa::Mutation> mutation) {
  if (!mutation) {
    throw std::invalid_argument("GRAPHQL_HTTP_PROCESSOR_MISSING_MUTATION");
  }
  return mutation;
}

std::shared_ptr<graphql::taraxa::Subscription> defaultGraphQlSubscription(
    std::shared_ptr<graphql::taraxa::Subscription> subscription) {
  if (subscription) {
    return subscription;
  }
  return std::make_shared<graphql::taraxa::Subscription>();
}
}  // namespace

GraphQlHttpProcessor::GraphQlHttpProcessor(GraphQlOperations operations)
    : HttpProcessor(),
      query_(requireGraphQlQuery(std::move(operations.query))),
      mutation_(requireGraphQlMutation(std::move(operations.mutation))),
      subscription_(defaultGraphQlSubscription(std::move(operations.subscription))),
      operations_(query_, mutation_, subscription_) {}

GraphQlHttpProcessor::GraphQlHttpProcessor(std::shared_ptr<::taraxa::final_chain::FinalChain> final_chain,
                                           std::shared_ptr<::taraxa::DagManager> dag_manager,
                                           std::shared_ptr<::taraxa::PbftManager> pbft_manager,
                                           std::shared_ptr<::taraxa::TransactionManager> transaction_manager,
                                           std::shared_ptr<::taraxa::DbStorage> db,
#ifdef RUSTAXA_ENABLE
                                           graphql::taraxa::QueryGasPriceReader gas_price_reader,
#else
                                           std::shared_ptr<::taraxa::GasPricer> gas_pricer,
#endif
                                           std::weak_ptr<::taraxa::Network> network, uint64_t chain_id,
                                           ::taraxa::net::LiveStatusReader live_status)
    : GraphQlHttpProcessor(GraphQlOperations{
          std::make_shared<graphql::taraxa::Query>(std::move(final_chain), std::move(dag_manager),
                                                   std::move(pbft_manager), transaction_manager, std::move(db),
#ifdef RUSTAXA_ENABLE
                                                   std::move(gas_price_reader),
#else
                                                   std::move(gas_pricer),
#endif
                                                   std::move(network), chain_id, std::move(live_status)),
          std::make_shared<graphql::taraxa::Mutation>(std::move(transaction_manager)),
          std::make_shared<graphql::taraxa::Subscription>()}) {
}

HttpProcessor::Response GraphQlHttpProcessor::process(const Request& request) {
  try {
    const std::string& request_str = request.body();

    peg::ast query_ast;
    response::Value variables{response::Type::Map};
    std::string operation_name{""};

    // Differenciate content type: 'application/json' vs 'application/graphql'
    // according to https://graphql.org/learn/serving-over-http/
    if (request.count("Content-Type") && request["Content-Type"] == "application/graphql") {
      query_ast = peg::parseString(request_str);
    } else {  // default is "application/json"

      Json::Value json;
      try {
        json = util::parse_json(request_str);
      } catch (Json::Exception const& e) {
        return createErrResponse("Unable to parse request json data");
      }

      const std::string query_key{service::strQuery};
      if (!json.isMember(query_key)) {
        return createErrResponse("Invalid request json data: Missing \'query\' in graphql request");
      }
      query_ast = peg::parseString(json[query_key].asString());

      // operationName field is optional
      if (json.isMember("operationName")) {
        operation_name = json["operationName"].asString();
      }

      // variables field is optional
      if (json.isMember("variables")) {
        variables = response::parseJSON(json["variables"].toStyledString());
        if (variables.type() != response::Type::Map) {
          return createErrResponse("Invalid request  json data: Variables object must be of map type");
        }
      }
    }

    auto result = operations_.resolve({query_ast, operation_name, std::move(variables), {}, nullptr}).get();
    return createOkResponse(response::toJSON(std::move(result)));

  } catch (const Json::Exception& e) {
    return createErrResponse("Unable to process request json data: " + std::string(e.what()));
  } catch (service::schema_exception& e) {
    return createErrResponse(e.getErrors());
  } catch (const std::exception& e) {
    return createErrResponse("Unable to process request: " + std::string(e.what()));
  }
}

HttpProcessor::Response GraphQlHttpProcessor::createErrResponse(std::string&& response_body) {
  // std::string graphql_er_json = "{\"data\":null,\"errors\":[{\"message\":\"" + response_body + "\"}]}";
  response::Value error(response::Type::Map);
  error.emplace_back(std::string{service::strMessage}, response::Value(std::move(response_body)));

  response::Value errors(response::Type::List);
  errors.emplace_back(std::move(error));

  response::Value response(response::Type::Map);
  response.emplace_back(std::string{service::strData}, response::Value());
  response.emplace_back(std::string{service::strErrors}, std::move(errors));

  return createOkResponse(response::toJSON(std::move(response)));
}

HttpProcessor::Response GraphQlHttpProcessor::createErrResponse(response::Value&& error_value) {
  response::Value response(response::Type::Map);
  response.emplace_back(std::string{service::strData}, response::Value());
  response.emplace_back(std::string{service::strErrors}, std::move(error_value));

  return createOkResponse(response::toJSON(std::move(response)));
}

HttpProcessor::Response GraphQlHttpProcessor::createOkResponse(std::string&& response_body) {
  Response response;
  response.set("Content-Type", "application/json");
  response.set("Access-Control-Allow-Origin", "*");
  response.set("Access-Control-Allow-Headers", "Accept, Accept-Language, Content-Language, Content-Type");
  response.set("Connection", "close");
  response.result(boost::beast::http::status::ok);
  response.body() = std::move(response_body);
  response.prepare_payload();

  return response;
}

}  // namespace taraxa::net
