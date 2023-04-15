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

#include "gflags/gflags.h"

namespace utils::bolt {

using namespace std::string_literals;

mg_memory::MgSessionPtr MakeBoltSession(const Config &config) {
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

}  // namespace utils::bolt
