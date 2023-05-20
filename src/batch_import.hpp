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

#include "utils/bolt.hpp"

// NOTE: Batched and parallel execution has many practical issue.
//   * In the transactional mode, there are many serialization errors -> check if a transaction was successfully
//     executed + retry are required.
//   * In the analytical mode, almost any query will pass (e.g. edge creation won't fail if nodes are not there) / it's
//     hard to detect any issue -> ordering of nodes and edges is the only way to correctly import data.

namespace mode::batch_import {

int Run(const utils::bolt::Config &bolt_config, int batch_size, int workers_number);

}  // namespace mode::batch_import
