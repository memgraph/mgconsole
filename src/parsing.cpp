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

#include "parsing.hpp"

#include "utils/utils.hpp"

namespace mode::parsing {

using namespace std::string_literals;

int Run() {
  int64_t query_index = 0;
  while (true) {
    auto query = query::GetQuery(nullptr);
    if (!query) {
      break;
    }
    if (query->empty()) {
      continue;
    }
    ++query_index;
  }
  std::cout << "Parsed " << query_index << " queries" << std::endl;
  return 0;
}

}  // namespace mode::parsing