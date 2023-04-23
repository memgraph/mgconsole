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

#include "iostream"

// Let's have a party with simple state machines!
// A more modular choice would be some lexer analyzer or Antlr, but that would add a lot of build complexity + would
// likely be slower.

namespace query::line {

struct CollectedClauses {
  bool has_match{false};
  bool has_create{false};
  bool has_merge{false};
};

enum class ClauseState { NONE, C, CR, CRE, CREA, CREAT, CREATE, M, MA, MAT, MATC, MATCH, ME, MER, MERG, MERGE };

inline ClauseState NextState(char *quote, char c, const ClauseState state) {
  if (c == ' ' || c == '\t') {
    return state;
  }

  if (!*quote && (c == 'C' || c == 'c') && state == ClauseState::NONE) {
    return ClauseState::C;
  } else if (!*quote && (c == 'R' || c == 'r') && state == ClauseState::C) {
    return ClauseState::CR;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::CR) {
    return ClauseState::CRE;
  } else if (!*quote && (c == 'A' || c == 'a') && state == ClauseState::CRE) {
    return ClauseState::CREA;
  } else if (!*quote && (c == 'T' || c == 't') && state == ClauseState::CREA) {
    return ClauseState::CREAT;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::CREAT) {
    return ClauseState::CREATE;

  } else if (!*quote && (c == 'M' || c == 'm') && state == ClauseState::NONE) {
    return ClauseState::M;
  } else if (!*quote && (c == 'A' || c == 'a') && state == ClauseState::M) {
    return ClauseState::MA;
  } else if (!*quote && (c == 'T' || c == 't') && state == ClauseState::MA) {
    return ClauseState::MAT;
  } else if (!*quote && (c == 'C' || c == 'c') && state == ClauseState::MAT) {
    return ClauseState::MATC;
  } else if (!*quote && (c == 'H' || c == 'h') && state == ClauseState::MATC) {
    return ClauseState::MATCH;

  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::M) {
    return ClauseState::ME;
  } else if (!*quote && (c == 'R' || c == 'r') && state == ClauseState::ME) {
    return ClauseState::MER;
  } else if (!*quote && (c == 'G' || c == 'g') && state == ClauseState::MER) {
    return ClauseState::MERG;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::MERG) {
    return ClauseState::MERGE;

  } else {
    return ClauseState::NONE;
  }
}

inline std::ostream &operator<<(std::ostream &os, const ClauseState &s) {
  if (s == ClauseState::CREATE) {
    os << "ClauseState(CREATE)";
  } else if (s == ClauseState::MATCH) {
    os << "ClauseState(MATCH)";
  } else if (s == ClauseState::MERGE) {
    os << "ClauseState(MERGE)";
  } else {
    os << "ClauseState";
  }
  return os;
}

}  // namespace query::line
