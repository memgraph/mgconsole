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

#include "parameters.hpp"

#include "check.hpp"

using query::params::ParamCommand;
using query::params::ParamStore;
using query::params::ParseParamCommand;

// `:param <name> <expression>` is parsed into a Set command carrying the
// parameter name and the (verbatim) Cypher expression.
void set_command_carries_name_and_expression() {
  auto result = ParseParamCommand(":param x 1 + 2");
  CHECK(result.is_param_command);
  CHECK(result.command.has_value());
  CHECK(result.error.empty());
  if (result.command) {
    CHECK(result.command->kind == ParamCommand::Kind::kSet);
    CHECK_EQ(result.command->name, std::string{"x"});
    CHECK_EQ(result.command->expression, std::string{"1 + 2"});
  }
}

// `:params` lists all currently-set parameters.
void params_lists_all() {
  auto result = ParseParamCommand(":params");
  CHECK(result.is_param_command);
  CHECK(result.command.has_value());
  CHECK(result.error.empty());
  if (result.command) {
    CHECK(result.command->kind == ParamCommand::Kind::kList);
  }
}

// `:params clear` removes all parameters.
void params_clear_clears_all() {
  auto result = ParseParamCommand(":params clear");
  CHECK(result.is_param_command);
  CHECK(result.command.has_value());
  CHECK(result.error.empty());
  if (result.command) {
    CHECK(result.command->kind == ParamCommand::Kind::kClear);
  }
}

// A `:param` with no name, or a name but no expression, is a
// recognised-but-malformed command.
void set_without_name_or_expression_is_an_error() {
  auto no_name = ParseParamCommand(":param");
  CHECK(no_name.is_param_command);
  CHECK(!no_name.command.has_value());
  CHECK(!no_name.error.empty());

  auto no_expression = ParseParamCommand(":param x");
  CHECK(no_expression.is_param_command);
  CHECK(!no_expression.command.has_value());
  CHECK(!no_expression.error.empty());
}

// An unknown `:params` argument is a recognised-but-malformed command.
void params_with_unknown_argument_is_an_error() {
  auto result = ParseParamCommand(":params bogus");
  CHECK(result.is_param_command);
  CHECK(!result.command.has_value());
  CHECK(!result.error.empty());
}

// Lines that are not parameter commands are left for other handlers, including
// look-alikes such as `:paramfoo` that share the `:param` prefix.
void non_param_lines_are_ignored() {
  for (const char *line : {":help", ":quit", "MATCH (n) RETURN n;", ":paramfoo", ":paramsfoo", ""}) {
    auto result = ParseParamCommand(line);
    CHECK(!result.is_param_command);
    CHECK(!result.command.has_value());
  }
}

// Surrounding whitespace is trimmed from the name and expression, while
// whitespace inside the expression is preserved verbatim.
void whitespace_around_name_and_expression_is_trimmed() {
  auto result = ParseParamCommand("  :param   x   1 + 2   ");
  CHECK(result.command.has_value());
  if (result.command) {
    CHECK_EQ(result.command->name, std::string{"x"});
    CHECK_EQ(result.command->expression, std::string{"1 + 2"});
  }
}

// A freshly constructed store holds no parameters.
void new_store_is_empty() {
  ParamStore store;
  CHECK(store.Empty());
  CHECK_EQ(store.Size(), std::size_t{0});
}

namespace {
mg_memory::MgValuePtr IntValue(int64_t n) {
  return mg_memory::MakeCustomUnique<mg_value>(mg_value_make_integer(n));
}
}  // namespace

// Setting a parameter stores a value retrievable by name.
void set_then_get_returns_equal_value() {
  ParamStore store;
  auto value = IntValue(42);
  store.Set("x", value.get());
  CHECK(!store.Empty());
  CHECK_EQ(store.Size(), std::size_t{1});
  const mg_value *got = store.Get("x");
  CHECK(got != nullptr);
  if (got) {
    CHECK(mg_value_get_type(got) == MG_VALUE_TYPE_INTEGER);
    CHECK_EQ(mg_value_integer(got), int64_t{42});
  }
}

// Setting an existing name replaces its value rather than adding a duplicate.
void set_overwrites_existing_value() {
  ParamStore store;
  store.Set("x", IntValue(1).get());
  store.Set("x", IntValue(2).get());
  CHECK_EQ(store.Size(), std::size_t{1});
  CHECK(store.Get("x") != nullptr);
  if (store.Get("x")) {
    CHECK_EQ(mg_value_integer(store.Get("x")), int64_t{2});
  }
}

// Parameter names are listed in sorted order (for stable `:params` output).
void names_are_returned_sorted() {
  ParamStore store;
  store.Set("b", IntValue(1).get());
  store.Set("a", IntValue(2).get());
  store.Set("c", IntValue(3).get());
  CHECK(store.Names() == (std::vector<std::string>{"a", "b", "c"}));
}

// Clear removes every parameter.
void clear_empties_store() {
  ParamStore store;
  store.Set("x", IntValue(1).get());
  store.Set("y", IntValue(2).get());
  store.Clear();
  CHECK(store.Empty());
  CHECK_EQ(store.Size(), std::size_t{0});
}

// AsMap builds an mg_map carrying every stored parameter, keyed by name.
void as_map_contains_all_parameters() {
  ParamStore store;
  store.Set("x", IntValue(42).get());
  store.Set("y", IntValue(7).get());

  auto map = store.AsMap();
  CHECK(map != nullptr);
  if (map) {
    CHECK_EQ(mg_map_size(map.get()), uint32_t{2});
    const mg_value *x = mg_map_at(map.get(), "x");
    const mg_value *y = mg_map_at(map.get(), "y");
    CHECK(x != nullptr && mg_value_integer(x) == 42);
    CHECK(y != nullptr && mg_value_integer(y) == 7);
  }
}

// An empty store still produces a usable (empty) mg_map.
void as_map_of_empty_store_is_empty() {
  ParamStore store;
  auto map = store.AsMap();
  CHECK(map != nullptr);
  if (map) CHECK_EQ(mg_map_size(map.get()), uint32_t{0});
}

int main() {
  RUN(set_command_carries_name_and_expression);
  RUN(params_lists_all);
  RUN(params_clear_clears_all);
  RUN(set_without_name_or_expression_is_an_error);
  RUN(params_with_unknown_argument_is_an_error);
  RUN(non_param_lines_are_ignored);
  RUN(whitespace_around_name_and_expression_is_trimmed);
  RUN(new_store_is_empty);
  RUN(set_then_get_returns_equal_value);
  RUN(set_overwrites_existing_value);
  RUN(names_are_returned_sorted);
  RUN(clear_empties_store);
  RUN(as_map_contains_all_parameters);
  RUN(as_map_of_empty_store_is_empty);
  return check::summary();
}
