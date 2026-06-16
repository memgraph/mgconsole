// Copyright (C) 2016-2023 Memgraph Ltd. [https://memgraph.com]
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

#include "bolt.hpp"

#include <optional>
#include <string>

#include "gflags/gflags.h"

namespace utils::bolt {

using namespace std::string_literals;

namespace {

// Opens a direct connection to config.host:config.port (the same logic that
// MakeBoltSession has always used). Returns a null MgSessionPtr on failure.
mg_memory::MgSessionPtr MakeDirectSession(const Config &config) {
  std::string bolt_client_version = "mg/"s + gflags::VersionString();
  mg_memory::MgSessionParamsPtr params = mg_memory::MakeCustomUnique<mg_session_params>(mg_session_params_make());
  if (!params) {
    console::EchoFailure("Connection failure", "out of memory, failed to allocate `mg_session_params` struct");
  }
  mg_session_params_set_host(params.get(), config.host.c_str());
  mg_session_params_set_port(params.get(), config.port);
  if (!config.username.empty()) {
    mg_session_params_set_username(params.get(), config.username.c_str());
    mg_session_params_set_password(params.get(), config.password.c_str());
  }
  mg_session_params_set_user_agent(params.get(), bolt_client_version.c_str());
  mg_session_params_set_sslmode(params.get(), config.use_ssl ? MG_SSLMODE_REQUIRE : MG_SSLMODE_DISABLE);
  mg_memory::MgSessionPtr session = mg_memory::MakeCustomUnique<mg_session>(nullptr);
  {
    mg_session *session_tmp;
    int status = mg_connect(params.get(), &session_tmp);
    session = mg_memory::MakeCustomUnique<mg_session>(session_tmp);
    if (status != 0) {
      console::EchoFailure("Connection failure", mg_session_error(session.get()));
      return mg_memory::MakeCustomUnique<mg_session>(nullptr);
    }
    return session;
  }
  return session;
}

// Splits a "host:port" address on its last ':' so IPv6 / hostnames with
// embedded colons still parse. Returns std::nullopt on a malformed address.
std::optional<std::pair<std::string, int>> SplitHostPort(const std::string &address) {
  const auto colon = address.rfind(':');
  if (colon == std::string::npos || colon == 0 || colon + 1 >= address.size()) {
    return std::nullopt;
  }
  const std::string host = address.substr(0, colon);
  const std::string port_str = address.substr(colon + 1);
  try {
    size_t consumed = 0;
    const int port = std::stoi(port_str, &consumed);
    if (consumed != port_str.size() || port <= 0 || port > 65535) {
      return std::nullopt;
    }
    return std::make_pair(host, port);
  } catch (const std::exception &) {
    return std::nullopt;
  }
}

// Reads an mg_string value into a std::string (mg_string is not null-terminated).
std::string ToString(const mg_string *str) {
  if (!str) {
    return {};
  }
  return std::string(mg_string_data(str), mg_string_size(str));
}

// Eagerly checks that `db` can be selected on `session` by running a throwaway query that carries it, so an
// unknown database fails at connect time instead of on the user's first query. Returns false (after reporting)
// ONLY when the database itself is unknown. Any other rejection is not a database problem and must not abort
// the connection: most importantly, a direct connection to a coordinator rejects all Cypher ("Coordinator can
// run only coordinator queries!"), yet must remain usable for coordinator commands.
bool ValidateDatabase(mg_session *session, const std::string &db) {
  if (db.empty()) {
    return true;
  }
  try {
    query::ExecuteQuery(session, "RETURN 1", nullptr, db);
    return true;
  } catch (const utils::ClientQueryException &e) {
    if (utils::ToLowerCase(e.what()).find("unknown database") != std::string::npos) {
      console::EchoFailure("Database selection failure", e.what());
      return false;
    }
    // Not a database error (e.g. a coordinator that can't run Cypher); let the connection proceed.
    return true;
  } catch (const utils::ClientFatalException &e) {
    console::EchoFailure("Connection failure", e.what());
    return false;
  }
}

}  // namespace

mg_memory::MgSessionPtr MakeBoltSession(const Config &config) {
  auto session = MakeDirectSession(config);
  if (session.get() != nullptr && !ValidateDatabase(session.get(), config.db)) {
    return mg_memory::MakeCustomUnique<mg_session>(nullptr);
  }
  return session;
}

// Currently, it sends both write and read queries to the current main
mg_memory::MgSessionPtr MakeRoutedBoltSession(const Config &config, std::chrono::steady_clock::time_point *expiry_out) {
  // 1. Connect to the coordinator.
  Config coord_config = config;
  coord_config.routed_connection = false;
  auto coord = MakeDirectSession(coord_config);
  if (coord.get() == nullptr) {
    return mg_memory::MakeCustomUnique<mg_session>(nullptr);
  }

  // 2. Build the routing context and extra map, then send the ROUTE message.
  const std::string coord_address = config.host + ":" + std::to_string(config.port);
  mg_memory::MgMapPtr routing = mg_memory::MakeCustomUnique<mg_map>(mg_map_make_empty(1));
  mg_memory::MgMapPtr extra = mg_memory::MakeCustomUnique<mg_map>(mg_map_make_empty(1));
  if (!routing || !extra) {
    console::EchoFailure("Routing failure", "out of memory, failed to allocate routing maps");
    return mg_memory::MakeCustomUnique<mg_session>(nullptr);
  }
  mg_map_insert(routing.get(), "address", mg_value_make_string(coord_address.c_str()));
  if (!config.db.empty()) {
    mg_map_insert(extra.get(), "db", mg_value_make_string(config.db.c_str()));
  }

  mg_map *rt_raw = nullptr;
  const int status = mg_session_route(coord.get(), routing.get(), nullptr, extra.get(), &rt_raw);
  // Take ownership immediately so the routing table is always freed.
  mg_memory::MgMapPtr rt = mg_memory::MakeCustomUnique<mg_map>(rt_raw);
  if (status != 0) {
    console::EchoFailure("Routing failure", mg_session_error(coord.get()));
    return mg_memory::MakeCustomUnique<mg_session>(nullptr);
  }

  // 3. Parse the routing table: TTL (seconds) and the WRITE (main) instance.
  const mg_value *ttl_val = mg_map_at(rt.get(), "ttl");
  if (ttl_val != nullptr && mg_value_get_type(ttl_val) == MG_VALUE_TYPE_INTEGER && expiry_out != nullptr) {
    *expiry_out = std::chrono::steady_clock::now() + std::chrono::seconds(mg_value_integer(ttl_val));
  }

  const mg_value *servers_val = mg_map_at(rt.get(), "servers");
  if (servers_val == nullptr) {
    console::EchoFailure("Routing failure", "routing table has no 'servers' entry");
    return mg_memory::MakeCustomUnique<mg_session>(nullptr);
  }
  const mg_list *servers = mg_value_list(servers_val);
  std::optional<std::string> write_address;
  bool has_router = false;
  for (uint32_t i = 0; i < mg_list_size(servers); ++i) {
    const mg_value *server_val = mg_list_at(servers, i);
    const mg_map *server = mg_value_map(server_val);
    if (server == nullptr) {
      continue;
    }
    const mg_value *role_val = mg_map_at(server, "role");
    if (role_val == nullptr || ToString(mg_value_string(role_val)) != "WRITE") {
      continue;
    }
    const mg_value *addresses_val = mg_map_at(server, "addresses");
    if (addresses_val == nullptr) {
      continue;
    }
    const mg_list *addresses = mg_value_list(addresses_val);
    if (mg_list_size(addresses) == 0) {
      continue;
    }
    write_address = ToString(mg_value_string(mg_list_at(addresses, 0)));
    break;
  }

  if (!write_address || write_address->empty()) {
    console::EchoFailure("Routing failure", "no WRITE (main) instance in routing table");
    return mg_memory::MakeCustomUnique<mg_session>(nullptr);
  }

  const auto host_port = SplitHostPort(*write_address);
  if (!host_port) {
    console::EchoFailure("Routing failure", "could not parse WRITE instance address '" + *write_address + "'");
    return mg_memory::MakeCustomUnique<mg_session>(nullptr);
  }

  // 4. Connect directly to the resolved main and drop the coordinator session. This goes through
  // MakeBoltSession (not MakeDirectSession) so the configured db is eagerly validated on the main, which runs
  // Cypher. The first/coordinator session above is intentionally never probed (coordinators reject Cypher).
  Config main_config = config;
  main_config.host = host_port->first;
  main_config.port = host_port->second;
  main_config.routed_connection = false;
  return MakeBoltSession(main_config);
}

RoutedSession::RoutedSession(Config config)
    : config_(std::move(config)),
      session_(mg_memory::MakeCustomUnique<mg_session>(nullptr)),
      routed_(config_.routed_connection) {
  Rebuild();
}

void RoutedSession::Rebuild() {
  if (routed_) {
    session_ = MakeRoutedBoltSession(config_, &expiry_);
  } else {
    // MakeBoltSession (not MakeDirectSession) so the configured db is eagerly validated on the direct path too.
    session_ = MakeBoltSession(config_);
  }
}

mg_session *RoutedSession::Get() {
  // Proactive TTL re-route: once the previous routing table has expired, fetch a fresh one (and possibly a new
  // main) before handing back the session. Suppressed while an explicit transaction is open (re-routing would
  // silently drop it) and made non-destructive: a failed re-route keeps the existing working session rather than
  // discarding a healthy connection over a transient coordinator hiccup.
  if (routed_ && !in_transaction_ && std::chrono::steady_clock::now() >= expiry_) {
    auto refreshed = MakeRoutedBoltSession(config_, &expiry_);
    if (refreshed.get() != nullptr || session_.get() == nullptr) {
      session_ = std::move(refreshed);
    } else {
      // Keep the still-usable session; back off so we retry periodically instead of on every call.
      expiry_ = std::chrono::steady_clock::now() + kRerouteRetryBackoffSec;
    }
  }
  return session_.get();
}

void RoutedSession::Reconnect() { Rebuild(); }

bool RoutedSession::Connected() const { return session_.get() != nullptr; }

void RoutedSession::ObserveQuery(const std::string &query) {
  // Track explicit-transaction state from the transaction-control keyword so proactive re-routing can avoid
  // tearing down an open transaction. Best-effort: matches the leading keyword of the trimmed query.
  const auto upper = utils::ToUpperCase(utils::Trim(query));
  if (upper.rfind("BEGIN", 0) == 0) {
    in_transaction_ = true;
  } else if (upper.rfind("COMMIT", 0) == 0 || upper.rfind("ROLLBACK", 0) == 0) {
    in_transaction_ = false;
  }
}

}  // namespace utils::bolt
