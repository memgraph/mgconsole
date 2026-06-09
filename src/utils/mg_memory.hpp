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

#include <memory>

#include "mgclient.h"

namespace mg_memory {
/// Unique pointers with custom deleters for automatic memory management of
/// mg_values.

template <class T>
inline void CustomDelete(T *);

template <>
inline void CustomDelete(mg_session *session) {
  mg_session_destroy(session);
}

template <>
inline void CustomDelete(mg_session_params *session_params) {
  mg_session_params_destroy(session_params);
}

template <>
inline void CustomDelete(mg_value *value) {
  mg_value_destroy(value);
}

template <>
inline void CustomDelete(mg_list *list) {
  mg_list_destroy(list);
}

template <>
inline void CustomDelete(mg_map *map) {
  mg_map_destroy(map);
}

template <class T>
using CustomUniquePtr = std::unique_ptr<T, void (*)(T *)>;

template <class T>
CustomUniquePtr<T> MakeCustomUnique(T *ptr) {
  return CustomUniquePtr<T>(ptr, CustomDelete<T>);
}

using MgSessionPtr = CustomUniquePtr<mg_session>;
using MgSessionParamsPtr = CustomUniquePtr<mg_session_params>;
using MgValuePtr = CustomUniquePtr<mg_value>;
using MgListPtr = CustomUniquePtr<mg_list>;
using MgMapPtr = CustomUniquePtr<mg_map>;

}  // namespace mg_memory
