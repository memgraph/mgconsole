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

#pragma once

#include <chrono>

#include "utils.hpp"

namespace utils::bolt {

struct Config {
  std::string db;
  std::string host;
  std::string username;
  std::string password;
  int port;
  bool use_ssl;
  bool routed_connection{false};  // when true, uses routing from coordinators
};

// Connects directly to config.host:config.port. Returns a null MgSessionPtr on
// failure (after reporting via console::EchoFailure).
mg_memory::MgSessionPtr MakeBoltSession(const Config &config);

// Connects to the coordinator at config.host:config.port, fetches the routing
// table via a Bolt ROUTE message, locates the WRITE (main) data instance and
// returns a direct session to it. On success, *expiry_out is set to the point
// in time at which the routing table's TTL expires. Returns a null MgSessionPtr
// on failure (after reporting via console::EchoFailure).
mg_memory::MgSessionPtr MakeRoutedBoltSession(const Config &config, std::chrono::steady_clock::time_point *expiry_out);

// A session abstraction that hides whether the connection is direct or routed.
// In routed mode it transparently re-fetches the routing table (re-routes)
// once the previous routing table's TTL has expired, keeping both connection
// modes on a single code path for the caller.
class RoutedSession {
 public:
  explicit RoutedSession(Config config);

  // Returns the underlying session. In routed mode, if the routing table TTL
  // has expired (and no explicit transaction is open), transparently re-routes
  // first; a failed re-route keeps the existing session. May return nullptr if
  // a (re-)connection attempt failed and there is no prior session to fall back
  // on.
  mg_session *Get();

  // Forces a rebuild of the session (direct or routed per config). Used by the
  // interactive fatal-error path to drive failover.
  void Reconnect();

  // True if a session is currently established.
  bool Connected() const;

  // Observes a query about to be executed so the session can track explicit
  // transaction boundaries (BEGIN/COMMIT/ROLLBACK) and suppress proactive
  // re-routing while a transaction is open. Safe to call in direct mode (no-op
  // effect on routing).
  void ObserveQuery(const std::string &query);

 private:
  // Backoff applied after a failed proactive re-route so we don't retry on
  // every Get() while the coordinator is briefly unreachable.
  static constexpr std::chrono::seconds kRerouteRetryBackoffSec{2};

  void Rebuild();

  Config config_;
  mg_memory::MgSessionPtr session_;
  std::chrono::steady_clock::time_point expiry_{};
  bool routed_;
  bool in_transaction_{false};
};

}  // namespace utils::bolt
