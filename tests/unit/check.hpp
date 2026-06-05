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

// Minimal dependency-free check harness for mgconsole unit tests.
//
// Usage:
//   #include "check.hpp"
//   void my_test() { CHECK(1 + 1 == 2); CHECK_EQ(answer, 42); }
//   int main() { RUN(my_test); return check::summary(); }

#include <cstdio>
#include <string>

namespace check {

inline int &failures() {
  static int n = 0;
  return n;
}

inline const char *&current_test() {
  static const char *name = "";
  return name;
}

inline void fail(const char *file, int line, const std::string &expr) {
  std::printf("  FAIL [%s] %s:%d: %s\n", current_test(), file, line, expr.c_str());
  ++failures();
}

inline int summary() {
  if (failures() == 0) {
    std::printf("All checks passed.\n");
    return 0;
  }
  std::printf("%d check(s) failed.\n", failures());
  return 1;
}

}  // namespace check

#define CHECK(cond)                                       \
  do {                                                    \
    if (!(cond)) ::check::fail(__FILE__, __LINE__, #cond); \
  } while (0)

#define CHECK_EQ(a, b)                                                                \
  do {                                                                                \
    auto &&_a = (a);                                                                  \
    auto &&_b = (b);                                                                  \
    if (!(_a == _b)) ::check::fail(__FILE__, __LINE__, #a " == " #b);                 \
  } while (0)

#define RUN(test_fn)                  \
  do {                                \
    ::check::current_test() = #test_fn; \
    test_fn();                        \
  } while (0)
