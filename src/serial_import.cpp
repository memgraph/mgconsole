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

#include "serial_import.hpp"

namespace mode::serial_import {

using namespace std::string_literals;

int Run(const utils::bolt::Config &bolt_config, const format::CsvOptions &csv_opts,
        const format::OutputOptions &output_opts) {
  auto session = MakeBoltSession(bolt_config);
  if (session.get() == nullptr) {
    return 1;
  }

  while (true) {
    auto query = query::GetQuery(nullptr);
    if (!query) {
      break;
    }
    if (query->empty()) {
      continue;
    }

    try {
      auto ret = query::ExecuteQuery(session.get(), *query);
      if (ret.records.size() > 0) {
        Output(ret.header, ret.records, output_opts, csv_opts);
      }
    } catch (const utils::ClientQueryException &e) {
      console::EchoFailure("Failed query", *query);
      console::EchoFailure("Client received query exception", e.what());
      return 1;
    } catch (const utils::ClientFatalException &e) {
      console::EchoFailure("Client received connection exception", e.what());
      return 1;
    }
  }

  return 0;
}

}  // namespace mode::serial_import
