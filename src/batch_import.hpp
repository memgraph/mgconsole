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

// NOTE: A big problem with batched execution is that executing batches with
//       edges will pass on an empty database (with no nodes) without an error:
//         * edges have to come after nodes
//         * count how many elements is actually created from a given batch
//
// TODO(gitbuda): If this is executed multiple times -> bad session + unknown message type -> debug
// TODO(gitbuda): The biggest issue seems to be that a few conflicting batches can end up in constant serialization
// conflict.
// TODO(gitbuda): Indexes are a problem because they can't be created in a multi-query transaction.
//     1. pre import -> indices -> serial
//     2. vertices -> only CREATE -> first but parallel
//     3. edges -> MATCH + CREATE -> after vertices
//     4. post import -> drop redunduntant indices & props -> serial
// TODO(gitbuda): Polish the implementation.

namespace mode::batch_import {

int Run(const utils::bolt::Config &bolt_config);

}  // namespace mode::batch_import
