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

int Run(bool collect_parsing_stats, bool print_parser_stats) {
  int64_t query_index = 0;
  while (true) {
    auto query = query::GetQuery(nullptr, collect_parsing_stats);
    if (!query) {
      break;
    }
    if (query->query.empty()) {
      continue;
    }
    if (collect_parsing_stats && print_parser_stats) {
      std::cout << "Line: " << query->line_number << " "
                << "Index: " << query->index << " "
                << "has_create: " << query->info->has_create << " "
                << "has_match: " << query->info->has_match << " "
                << "has_merge: " << query->info->has_merge << " "
                << "has_detach_delete: " << query->info->has_detach_delete << " "
                << "has_create_index: " << query->info->has_create_index << " "
                << "has_drop_index: " << query->info->has_drop_index << " "
                << "has_storage_mode: " << query->info->has_storage_mode << " "
                << "has_remove: " << query->info->has_remove << " " << std::endl;
    }
    ++query_index;
  }
  std::cout << "Parsed " << query_index << " queries" << std::endl;
  return 0;
}

}  // namespace mode::parsing
