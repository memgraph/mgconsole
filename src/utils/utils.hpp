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

#include <cstdint>
#include <filesystem>
#include <iostream>
#include <map>
#include <memory>
#include <optional>
#include <string>
#include <vector>

#ifdef _WIN32
#define STDIN_FILENO 0
#endif /* _WIN32 */

#include "mgclient.h"
#include "replxx.h"

#include "query_type.hpp"

namespace fs = std::filesystem;

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
using MgListPtr = CustomUniquePtr<mg_list>;
using MgMapPtr = CustomUniquePtr<mg_map>;

}  // namespace mg_memory

namespace utils {

class ClientFatalException : public std::exception {
 public:
  ClientFatalException(std::string what) : what_(std::move(what)) {}
  const char *what() const noexcept override { return what_.c_str(); }

 private:
  std::string what_;
};

class ClientQueryException : public std::exception {
 public:
  ClientQueryException(std::string what) : what_(std::move(what)) {}
  const char *what() const noexcept override { return what_.c_str(); }

 private:
  std::string what_;
};

bool EnsureDir(const fs::path &dir) noexcept;

fs::path GetUserHomeDir();

/**
 * return string with all uppercased characters (locale independent).
 */
std::string ToUpperCase(std::string s);

/**
 * removes whitespace characters from the start and from the end of a string.
 *
 * @param str string that is going to be trimmed
 *
 * @return trimmed string
 */
std::string Trim(const std::string &s);

/**
 * replaces all occurences of <match> in <src> with <replacement>.
 */
// todo: this could be implemented much more efficiently.
std::string Replace(std::string src, const std::string &match, const std::string &replacement);

/// escapes all whitespace and quotation characters to produce a string
/// which can be used as a string literal.
std::string Escape(const std::string &src);

/**
 * outputs a collection of items to the given stream, separating them with the
 * given delimiter.
 *
 * @param stream destination stream.
 * @param iterable an iterable collection of items.
 * @param delim delimiter that is put between items.
 * @param streamer function which accepts a tstream and an item and
 *  streams the item to the stream.
 */
template <typename tstream, typename titerable>
inline void PrintIterable(tstream &stream, const titerable &iterable, const std::string &delim = ", ") {
  bool first = true;
  for (const auto &item : iterable) {
    if (first)
      first = false;
    else
      stream << delim;
    stream << item;
  }
}

void PrintStringUnescaped(std::ostream &os, const mg_string *str);

void PrintValue(std::ostream &os, const mg_string *str);

void PrintValue(std::ostream &os, const mg_map *map);

void PrintValue(std::ostream &os, const mg_node *node);

void PrintValue(std::ostream &os, const mg_relationship *rel);

void PrintValue(std::ostream &os, const mg_unbound_relationship *rel);

void PrintValue(std::ostream &os, const mg_path *path);

void PrintValue(std::ostream &os, const mg_date *date);

void PrintValue(std::ostream &os, const mg_local_time *local_time);

void PrintValue(std::ostream &os, const mg_local_date_time *local_date_time);

void PrintValue(std::ostream &os, const mg_duration *duration);

void PrintValue(std::ostream &os, const mg_value *value);

}  // namespace utils

// Unfinished query text from previous input.
// e.g. Previous input was MATCH(n) RETURN n; MATCH
// then default_text would be set to MATCH for next query.
static std::string mgconsole_global_default_text;
// The following variables are used to track the line number and index (number specifying order) of the processed query.
[[maybe_unused]] static int64_t mgconsole_global_line_number{0};
[[maybe_unused]] static int64_t mgconsole_global_query_index{0};

namespace console {

bool is_a_tty(int fd);

void PrintHelp();

void PrintDocs();

void EchoFailure(const std::string &failure_msg, const std::string &explanation);

void EchoInfo(const std::string &message);

void EchoStats(const std::map<std::string, std::int64_t> &stats);

void EchoNotification(const std::map<std::string, std::string> &notification);

void EchoExecutionInfo(const std::map<std::string, double> &execution_info);

/// Helper function that sets default input for 'readline'
int SetDefaultText();

void SetStdinEcho(bool enable);

std::optional<std::string> GetLine();

struct ParseLineInfo {
  query::line::CollectedClauses collected_clauses;
};
struct ParseLineResult {
  std::string line;
  bool is_done;
  // In the case when caller is interested in more info.
  std::optional<ParseLineInfo> info;
};
/// Because query can span across multiple lines.
inline ParseLineInfo MergeParseLineInfo(const ParseLineInfo &l, const ParseLineInfo &r) {
  return ParseLineInfo{
      .collected_clauses = query::line::MergeCollectedClauses(l.collected_clauses, r.collected_clauses),
  };
}
/// Helper function that parses user line input.
/// @param line user input line.
/// @param quote quote character or '\0'; if set line is inside quotation.
/// @param escaped if set, next character should be escaped.
/// @return ParseLineResult a pair of string and bool. string is parsed line and bool marks
/// if query finished(Query finishes with ';') with this line. + optionally info about what line contains
ParseLineResult ParseLine(const std::string &line, char *quote, bool *escaped, bool collect_info = false);

/// Helper function that reads a line from the
/// standard input using the 'readline' lib.
/// Adds support for history and reverse-search.
/// @param prompt The prompt to display.
/// @return  User input line, or nullopt on EOF.
std::optional<std::string> ReadLine(Replxx *replxx_instance, const std::string &prompt);

}  // namespace console

namespace query {

// Interesting abstraction because multiple lines can be parsed in parallel.
struct Line {
  int64_t line_number;
  std::string line;
};

// NOTE: In theory it's possible to merge QueryInfo and CollectedClauses because they are the same, but it's not clear
// what would be the best, leaving as is.
struct QueryInfo {
  bool has_create{false};
  bool has_match{false};
  bool has_merge{false};
  bool has_detach_delete{false};
  bool has_create_index{false};
  bool has_drop_index{false};
  bool has_remove{false};
  bool has_storage_mode{false};
};

inline std::optional<QueryInfo> QueryInfoFromParseLineInfo(const std::optional<console::ParseLineInfo> &line_info) {
  // NOTE: The logic here is correct only if there is a controlled input, change to make batched and parallel import
  // non-experimental feature.
  if (line_info) {
    return QueryInfo{
        .has_create = line_info->collected_clauses.has_create,
        .has_match = line_info->collected_clauses.has_match,
        .has_merge = line_info->collected_clauses.has_merge,
        .has_detach_delete = line_info->collected_clauses.has_detach_delete,
        .has_create_index = line_info->collected_clauses.has_create_index,
        .has_drop_index = line_info->collected_clauses.has_drop_index,
        .has_remove = line_info->collected_clauses.has_remove,
        .has_storage_mode = line_info->collected_clauses.has_storage_mode,
    };
  } else {
    return std::nullopt;
  }
}

struct Query {
  int64_t line_number{0};
  int64_t index{0};
  std::string query{""};
  std::optional<QueryInfo> info{std::nullopt};
};
void PrintQueryInfo(const Query &);

struct Batch {
  explicit Batch(int64_t capacity, int64_t index) : capacity(capacity), index(index) { queries.reserve(capacity); }
  Batch() = delete;
  Batch(const Batch &) = delete;
  Batch &operator=(const Batch &) = delete;
  Batch(Batch &&) = default;
  Batch &operator=(Batch &&) = default;

  int64_t capacity;
  int64_t index;
  std::vector<Query> queries;
  bool is_executed = false;
  int64_t backoff = 1;
  int64_t attempts = 0;
};
void PrintBatchesInfo(const std::vector<Batch> &);

struct QueryResult {
  std::vector<std::string> header;
  std::vector<mg_memory::MgListPtr> records;
  std::chrono::duration<double> wall_time;
  std::optional<std::map<std::string, std::string>> notification;
  std::optional<std::map<std::string, std::int64_t>> stats;
  std::optional<std::map<std::string, double>> execution_info;
};

struct BatchResult {
  bool is_executed;
  std::vector<QueryResult> results;
};

// Depends on the global static string because of ...; MATCH
// The extra part is preserved for the next GetQuery call
std::optional<Query> GetQuery(Replxx *replxx_instance, bool collect_info = false);

QueryResult ExecuteQuery(mg_session *session, const std::string &query);
BatchResult ExecuteBatch(mg_session *session, const Batch &batch);

}  // namespace query

namespace format {

struct CsvOptions {
  CsvOptions(std::string delim, std::string escape, const bool dquote)
      : delimiter(std::move(delim)), escapechar(std::move(escape)), doublequote(dquote) {}

  bool ValidateDoubleQuote() {
    if (!doublequote && escapechar.size() != 1) {
      return false;
    }
    return true;
  }

  std::string delimiter;
  std::string escapechar;
  bool doublequote;
};

struct OutputOptions {
  OutputOptions(std::string out_format, const bool fit_to_scr)
      : output_format(std::move(out_format)), fit_to_screen(fit_to_scr) {}

  std::string output_format;
  bool fit_to_screen;
};

void PrintHeaderTabular(const std::vector<std::string> &data, int total_width, int column_width, int num_columns,
                        bool all_columns_fit, int margin);

/// Helper function for determining maximum length of data.
/// @param data List of mg_values representing row.
/// @param margin Column margin width.
/// @return length needed for representing max size element in @p data list.
/// Plus one is added because of column start character '|'.
uint64_t GetMaxColumnWidth(const mg_memory::MgListPtr &data, int margin);

uint64_t GetMaxColumnWidth(const std::vector<std::string> &data, int margin);

void PrintRowTabular(const mg_memory::MgListPtr &data, int total_width, int column_width, int num_columns,
                     bool all_columns_fit, int margin);

void PrintTabular(const std::vector<std::string> &header, const std::vector<mg_memory::MgListPtr> &records,
                  const bool fit_to_screen);

std::vector<std::string> FormatCsvFields(const mg_memory::MgListPtr &fields, const CsvOptions &csv_opts);

std::vector<std::string> FormatCsvHeader(const std::vector<std::string> &fields, const CsvOptions &csv_opts);

void PrintCsv(const std::vector<std::string> &header, const std::vector<mg_memory::MgListPtr> &records,
              const CsvOptions &csv_opts);

void Output(const std::vector<std::string> &header, const std::vector<mg_memory::MgListPtr> &records,
            const OutputOptions &out_opts, const CsvOptions &csv_opts);
}  // namespace format

Replxx *InitAndSetupReplxx();
