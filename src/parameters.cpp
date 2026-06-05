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

namespace query::params {

namespace {

constexpr const char *kWhitespace = " \t\r\n\f\v";

std::string Trim(const std::string &s) {
  const auto begin = s.find_first_not_of(kWhitespace);
  if (begin == std::string::npos) return "";
  const auto end = s.find_last_not_of(kWhitespace);
  return s.substr(begin, end - begin + 1);
}

}  // namespace

ParamParse ParseParamCommand(const std::string &line) {
  const std::string trimmed = Trim(line);
  const auto head_end = trimmed.find_first_of(kWhitespace);
  const std::string head = trimmed.substr(0, head_end);
  const std::string rest = head_end == std::string::npos ? "" : Trim(trimmed.substr(head_end));

  if (head == ":params") {
    if (rest.empty()) {
      ParamCommand command;
      command.kind = ParamCommand::Kind::kList;
      return {.is_param_command = true, .command = command, .error = ""};
    }
    if (rest == "clear") {
      ParamCommand command;
      command.kind = ParamCommand::Kind::kClear;
      return {.is_param_command = true, .command = command, .error = ""};
    }
    return {.is_param_command = true, .command = std::nullopt, .error = "expected ':params' or ':params clear'"};
  }

  if (head != ":param") return {};  // not a parameter command

  const auto name_end = rest.find_first_of(kWhitespace);
  ParamCommand command;
  command.kind = ParamCommand::Kind::kSet;
  command.name = rest.substr(0, name_end);
  command.expression = name_end == std::string::npos ? "" : Trim(rest.substr(name_end));

  if (command.name.empty() || command.expression.empty()) {
    return {.is_param_command = true, .command = std::nullopt, .error = "expected ':param <name> <expression>'"};
  }

  return {.is_param_command = true, .command = command, .error = ""};
}

bool ParamStore::Empty() const { return params_.empty(); }

std::size_t ParamStore::Size() const { return params_.size(); }

void ParamStore::Set(const std::string &name, const mg_value *value) {
  params_.insert_or_assign(name, mg_memory::MakeCustomUnique<mg_value>(mg_value_copy(value)));
}

const mg_value *ParamStore::Get(const std::string &name) const {
  const auto it = params_.find(name);
  return it == params_.end() ? nullptr : it->second.get();
}

std::vector<std::string> ParamStore::Names() const {
  std::vector<std::string> names;
  names.reserve(params_.size());
  for (const auto &[name, value] : params_) names.push_back(name);
  return names;  // std::map keeps keys sorted
}

void ParamStore::Clear() { params_.clear(); }

mg_memory::MgMapPtr ParamStore::AsMap() const {
  auto map = mg_memory::MakeCustomUnique<mg_map>(mg_map_make_empty(static_cast<uint32_t>(params_.size())));
  for (const auto &[name, value] : params_) {
    // mg_map_insert copies the key and takes ownership of the value copy.
    mg_map_insert(map.get(), name.c_str(), mg_value_copy(value.get()));
  }
  return map;
}

}  // namespace query::params
