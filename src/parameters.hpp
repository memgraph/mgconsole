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

#include <cstddef>
#include <map>
#include <optional>
#include <string>
#include <vector>

#include "utils/mg_memory.hpp"

namespace query::params {

/// A parsed `:param` / `:params` interactive command.
struct ParamCommand {
  enum class Kind {
    kSet,    ///< `:param <name> <expression>`
    kList,   ///< `:params`
    kClear,  ///< `:params clear`
  };

  Kind kind;
  std::string name;        ///< populated for kSet
  std::string expression;  ///< populated for kSet
};

/// Result of attempting to parse a line as a parameter command.
struct ParamParse {
  /// True if the line is a `:param`/`:params` command (well-formed or not).
  bool is_param_command{false};
  /// Set iff the command parsed successfully.
  std::optional<ParamCommand> command{std::nullopt};
  /// Human-readable reason, set iff `is_param_command && !command`.
  std::string error{};
};

/// Parses a single line as a parameter command.
///
/// Returns `is_param_command == false` for anything that is not a
/// `:param`/`:params` command, so other command handlers can take over.
ParamParse ParseParamCommand(const std::string &line);

/// Holds the query parameters set via `:param`, owning a copy of each value,
/// and exposes them as an `mg_map` for `mg_session_run`.
class ParamStore {
 public:
  bool Empty() const;
  std::size_t Size() const;

  /// Stores a copy of `value` under `name`, overwriting any existing entry.
  void Set(const std::string &name, const mg_value *value);
  /// Returns the value stored under `name`, or nullptr if none.
  const mg_value *Get(const std::string &name) const;
  /// Returns all parameter names in sorted order.
  std::vector<std::string> Names() const;
  /// Removes all parameters.
  void Clear();
  /// Builds an `mg_map` copy of all parameters, suitable for `mg_session_run`.
  mg_memory::MgMapPtr AsMap() const;

 private:
  std::map<std::string, mg_memory::MgValuePtr> params_;
};

}  // namespace query::params
