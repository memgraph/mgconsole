#pragma once

#include <cstdint>
#include <iostream>
#include <memory>

#include <filesystem>
#include <map>
#include <optional>
#include <string>
#include <vector>

#ifdef _WIN32
#define STDIN_FILENO 0
#endif /* _WIN32 */

#include "mgclient.h"
#include "replxx.h"

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
static std::string default_text;

namespace console {

bool is_a_tty(int fd);

void PrintHelp();

void PrintDocs();

void EchoFailure(const std::string &failure_msg, const std::string &explanation);

void EchoInfo(const std::string &message);

void EchoStats(const std::map<std::string, std::int64_t> &stats);

void EchoNotification(const std::map<std::string, std::string> &notification);

void EchoExecutionTimeInfo(const std::map<std::string, double> &execution_info);

/// Helper function that sets default input for 'readline'
int SetDefaultText();

void SetStdinEcho(bool enable);

std::optional<std::string> GetLine();

/// Helper function that parses user line input.
/// @param line user input line.
/// @param quote quote character or '\0'; if set line is inside quotation.
/// @param escaped if set, next character should be escaped.
/// @return pair of string and bool. string is parsed line and bool marks
/// if query finished(Query finishes with ';') with this line.
std::pair<std::string, bool> ParseLine(const std::string &line, char *quote, bool *escaped);

/// Helper function that reads a line from the
/// standard input using the 'readline' lib.
/// Adds support for history and reverse-search.
/// @param prompt The prompt to display.
/// @return  User input line, or nullopt on EOF.
std::optional<std::string> ReadLine(Replxx *replxx_instance, const std::string &prompt);

}  // namespace console

namespace query {

struct QueryData {
  std::vector<std::string> header;
  std::vector<mg_memory::MgListPtr> records;
  std::chrono::duration<double> wall_time;
  std::optional<std::map<std::string, std::string>> notification;
  std::optional<std::map<std::string, std::int64_t>> stats;
  std::optional<std::map<std::string, double>> execution_time_info;
};

std::optional<std::string> GetQuery(Replxx *replxx_instance);

QueryData ExecuteQuery(mg_session *session, const std::string &query);

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
