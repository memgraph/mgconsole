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

// Let's have a party with simple state machine! The intention here is to
// implement something simple to understand and fast; to experiment with the
// batched and parallelization execution mode.
// A more modular choice would be some lexer analyzer or Antlr, but that would
// add a lot of build complexity + would likely be slower.
// Consider implementing some simple but fast state machine or at least try to
// be DRY (in this case makes a lot of sense).

namespace query::line {

struct CollectedClauses {
  bool has_match{false};
  bool has_create{false};
  bool has_merge{false};
  bool has_create_index{false};
  bool has_detach_delete{false};
  bool has_remove{false};
  bool has_drop_index{false};
};

inline std::ostream &operator<<(std::ostream &os, const CollectedClauses &cc) {
  os << "CollectedClauses: ";
  if (cc.has_match) {
    os << "HAS_MATCH ";
  } else if (cc.has_create) {
    os << "HAS_CREATE ";
  } else if (cc.has_merge) {
    os << "HAS_MERGE ";
  } else if (cc.has_create_index) {
    os << "HAS_CREATE_INDEX ";
  } else if (cc.has_detach_delete) {
    os << "HAS_DETACH_DELETE ";
  } else if (cc.has_remove) {
    os << "HAS_REMOVE ";
  } else if (cc.has_drop_index) {
    os << "HAS_DROP_INDEX ";
  }
  return os;
}

// clang-format off
enum class ClauseState {
   NONE,                                  // CREATE_(
   C, CR, CRE, CREA, CREAT, CREATE, CREATE_, CREATE_P,
                                             CREATE_I, CREATE_IN, CREATE_IND, CREATE_INDE, CREATE_INDEX,
   M, MA, MAT, MATC, MATCH,         MATCH_, MATCH_P,
      ME, MER, MERG, MERGE,         MERGE_, MERGE_P,
   D, DE, DET, DETA, DETAC, DETACH, DETACH_, DETACH_D, DETACH_DE, DETACH_DEL, DETACH_DELE, DETACH_DELET, DETACH_DELETE,
      DR, DRO, DROP, DROP_, DROP_I, DROP_IN, DROP_IND, DROP_INDE, DROP_INDEX,
// )_REMOVE
   P, P_, P_R, P_RE, P_REM, P_REMO, P_REMOV, P_REMOVE,
};
// clang-format on

inline bool IsWhitespace(char c) { return c == ' ' || c == '\t' || c == '\n'; }

inline ClauseState NextState(char *quote, char c, const ClauseState state) {
  if (!*quote && IsWhitespace(c)) {
    if (state == ClauseState::CREATE) {
      return ClauseState::CREATE_;
    } else if (state == ClauseState::MATCH) {
      return ClauseState::MATCH_;
    } else if (state == ClauseState::MERGE) {
      return ClauseState::MERGE_;
    } else if (state == ClauseState::DETACH) {
      return ClauseState::DETACH_;
    } else if (state == ClauseState::DROP) {
      return ClauseState::DROP_;
    } else if (state == ClauseState::P) {
      return ClauseState::P_;
    } else {
      return state;
    }
  }

  // CREATE
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
  } else if (!*quote && c == '(' && (state == ClauseState::CREATE || state == ClauseState::CREATE_)) {
    return ClauseState::CREATE_P;

    // CREATE INDEX
  } else if (!*quote && (c == 'I' || c == 'i') && state == ClauseState::CREATE_) {
    return ClauseState::CREATE_I;
  } else if (!*quote && (c == 'N' || c == 'n') && state == ClauseState::CREATE_I) {
    return ClauseState::CREATE_IN;
  } else if (!*quote && (c == 'D' || c == 'd') && state == ClauseState::CREATE_IN) {
    return ClauseState::CREATE_IND;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::CREATE_IND) {
    return ClauseState::CREATE_INDE;
  } else if (!*quote && (c == 'X' || c == 'x') && state == ClauseState::CREATE_INDE) {
    return ClauseState::CREATE_INDEX;

    // MATCH
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
  } else if (!*quote && c == '(' && (state == ClauseState::MATCH || state == ClauseState::MATCH_)) {
    return ClauseState::MATCH_P;

    // MERGE
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::M) {
    return ClauseState::ME;
  } else if (!*quote && (c == 'R' || c == 'r') && state == ClauseState::ME) {
    return ClauseState::MER;
  } else if (!*quote && (c == 'G' || c == 'g') && state == ClauseState::MER) {
    return ClauseState::MERG;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::MERG) {
    return ClauseState::MERGE;
  } else if (!*quote && c == '(' && (state == ClauseState::MERGE || state == ClauseState::MERGE_)) {
    return ClauseState::MERGE_P;

    // DETACH DELETE
  } else if (!*quote && (c == 'D' || c == 'd') && state == ClauseState::NONE) {
    return ClauseState::D;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::D) {
    return ClauseState::DE;
  } else if (!*quote && (c == 'T' || c == 't') && state == ClauseState::DE) {
    return ClauseState::DET;
  } else if (!*quote && (c == 'A' || c == 'a') && state == ClauseState::DET) {
    return ClauseState::DETA;
  } else if (!*quote && (c == 'C' || c == 'c') && state == ClauseState::DETA) {
    return ClauseState::DETAC;
  } else if (!*quote && (c == 'H' || c == 'h') && state == ClauseState::DETAC) {
    return ClauseState::DETACH;
  } else if (!*quote && (c == 'D' || c == 'd') && state == ClauseState::DETACH_) {
    return ClauseState::DETACH_D;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::DETACH_D) {
    return ClauseState::DETACH_DE;
  } else if (!*quote && (c == 'L' || c == 'e') && state == ClauseState::DETACH_DE) {
    return ClauseState::DETACH_DEL;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::DETACH_DEL) {
    return ClauseState::DETACH_DELE;
  } else if (!*quote && (c == 'T' || c == 'e') && state == ClauseState::DETACH_DELE) {
    return ClauseState::DETACH_DELET;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::DETACH_DELET) {
    return ClauseState::DETACH_DELETE;

    // DROP INDEX
  } else if (!*quote && (c == 'R' || c == 'r') && state == ClauseState::D) {
    return ClauseState::DR;
  } else if (!*quote && (c == 'O' || c == 'o') && state == ClauseState::DR) {
    return ClauseState::DRO;
  } else if (!*quote && (c == 'P' || c == 'p') && state == ClauseState::DRO) {
    return ClauseState::DROP;
  } else if (!*quote && (c == 'I' || c == 'i') && state == ClauseState::DROP_) {
    return ClauseState::DROP_I;
  } else if (!*quote && (c == 'N' || c == 'n') && state == ClauseState::DROP_I) {
    return ClauseState::DROP_IN;
  } else if (!*quote && (c == 'D' || c == 'd') && state == ClauseState::DROP_IN) {
    return ClauseState::DROP_IND;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::DROP_IND) {
    return ClauseState::DROP_INDE;
  } else if (!*quote && (c == 'X' || c == 'x') && state == ClauseState::DROP_INDE) {
    return ClauseState::DROP_INDEX;

    // ) REMOVE
  } else if (!*quote && c == ')' && state == ClauseState::NONE) {
    return ClauseState::P;
  } else if (!*quote && (c == 'R' || c == 'r') && state == ClauseState::P_) {
    return ClauseState::P_R;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::P_R) {
    return ClauseState::P_RE;
  } else if (!*quote && (c == 'M' || c == 'm') && state == ClauseState::P_RE) {
    return ClauseState::P_REM;
  } else if (!*quote && (c == 'O' || c == 'o') && state == ClauseState::P_REM) {
    return ClauseState::P_REMO;
  } else if (!*quote && (c == 'V' || c == 'v') && state == ClauseState::P_REMO) {
    return ClauseState::P_REMOV;
  } else if (!*quote && (c == 'E' || c == 'e') && state == ClauseState::P_REMOV) {
    return ClauseState::P_REMOVE;

  } else {
    return ClauseState::NONE;
  }
}

inline std::ostream &operator<<(std::ostream &os, const ClauseState &s) {
  if (s == ClauseState::CREATE_P) {
    os << "CREATE_(";
  } else if (s == ClauseState::MATCH_P) {
    os << "MATCH_(";
  } else if (s == ClauseState::MERGE_P) {
    os << "MERGE_(";
  } else if (s == ClauseState::CREATE_INDEX) {
    os << "CREATE_INDEX";
  } else if (s == ClauseState::DETACH_DELETE) {
    os << "DETACH_DELETE";
  } else if (s == ClauseState::DROP_INDEX) {
    os << "DROP_INDEX";
  } else if (s == ClauseState::P_REMOVE) {
    os << ")_REMOVE";
  } else {
    os << "Some ClauseState";
  }
  return os;
}

}  // namespace query::line
