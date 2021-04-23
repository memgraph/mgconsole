// mgconsole - console client for Memgraph database
// Copyright (C) 2016-2020 Memgraph Ltd. [https://memgraph.com]
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

#include <algorithm>
#include <experimental/filesystem>
#include <experimental/optional>
#include <iostream>
#include <thread>

#include <pwd.h>
#include <signal.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

#include <gflags/gflags.h>
#include <readline/history.h>
#include <readline/readline.h>

#include "mgclient.h"
#include "version.hpp"

namespace fs = std::experimental::filesystem;

using namespace std::string_literals;

volatile sig_atomic_t is_shutting_down = 0;

// Usage strings.
static const std::string kUsage =
    "Memgraph bolt client.\n"
    "The client can be run in interactive or non-interactive mode.\n";
static const std::string kInteractiveUsage =
    "In interactive mode, user can enter cypher queries and supported "
    "commands.\n\n"
    "Cypher queries can span through multiple lines and conclude with a\n"
    "semi-colon (;). Each query is executed in the database and the results\n"
    "are printed out.\n\n"
    "The following interactive commands are supported:\n\n"
    "\t:help\t Print out usage for interactive mode\n"
    "\t:quit\t Exit the shell\n";

// Supported commands.
// Maybe also add reconnect?
static const std::string kCommandQuit = ":quit";
static const std::string kCommandHelp = ":help";

// Supported formats.
static const std::string kCsvFormat = "csv";
static const std::string kTabularFormat = "tabular";

DEFINE_string(host, "127.0.0.1",
              "Server address. It can be a DNS resolvable hostname.");
DEFINE_int32(port, 7687, "Server port");
DEFINE_string(username, "", "Username for the database");
DEFINE_string(password, "", "Password for the database");
DEFINE_bool(use_ssl, false, "Use SSL when connecting to the server.");
DEFINE_bool(fit_to_screen, false, "Fit output width to screen width.");
DEFINE_string(output_format, "tabular",
              "Query output format. Can be csv/tabular. If output format is "
              "other than tabular `fit-to-screen` flag is ignored.");
DEFINE_validator(output_format, [](const char *, const std::string &value) {
  if (value == kCsvFormat || value == kTabularFormat) {
    return true;
  }
  return false;
});
DEFINE_string(csv_delimiter, ",", "Character used to separate fields.");
DEFINE_validator(csv_delimiter, [](const char *, const std::string &value) {
  if (value.size() != 1) {
    return false;
  }
  return true;
});
DEFINE_string(
    csv_escapechar, "",
    "Character used to escape the quotechar(\") if csv-doublequote is false.");
DEFINE_bool(
    csv_doublequote, true,
    "Controls how instances of quotechar(\") appearing inside a field should "
    "themselves be quoted. When true, the character is doubled. When false, "
    "the escapechar is used as a prefix to the quotechar. "
    "If csv-doublequote is false 'csv-escapechar' must be set.");

static bool ValidateCsvDoubleQuote() {
  if (!FLAGS_csv_doublequote && FLAGS_csv_escapechar.size() != 1) {
    return false;
  }
  return true;
}

DEFINE_string(history, "~/.memgraph",
              "Use the specified directory for saving history.");
DEFINE_bool(no_history, false, "Do not save history.");

// History default directory.
static const std::string kDefaultHistoryBaseDir = "~";
static const std::string kDefaultHistoryMemgraphDir = ".memgraph";
// History filename.
static const std::string kHistoryFilename = "client_history";

DECLARE_int32(min_log_level);

// Unfinished query text from previous input.
// e.g. Previous input was MATCH(n) RETURN n; MATCH
// then default_text would be set to MATCH for next query.
static std::string default_text;

static const std::string kPrompt = "memgraph> ";
static const std::string kMultilinePrompt = "       -> ";

static void PrintHelp() { std::cout << kInteractiveUsage << std::endl; }

namespace wrap {
/// Unique pointers with custom deleters for automatic memory management of
/// mg_values.

template <class T>
void CustomDelete(T *);

template <>
void CustomDelete(mg_session *session) {
  mg_session_destroy(session);
}

template <>
void CustomDelete(mg_session_params *session_params) {
  mg_session_params_destroy(session_params);
}

template <>
void CustomDelete(mg_list *list) {
  mg_list_destroy(list);
}

template <>
void CustomDelete(mg_map *map) {
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

}  // namespace wrap

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

bool EnsureDir(const fs::path &dir) noexcept {
  std::error_code error_code;  // For exception suppression.
  if (fs::exists(dir, error_code)) return fs::is_directory(dir, error_code);
  return fs::create_directories(dir, error_code);
}

/**
 * Return string with all uppercased characters (locale independent).
 */
inline std::string ToUpperCase(std::string s) {
  std::transform(s.begin(), s.end(), s.begin(),
                 [](char c) { return toupper(c); });
  return s;
}

/**
 * Removes whitespace characters from the start and from the end of a string.
 *
 * @param str string that is going to be trimmed
 *
 * @return trimmed string
 */
inline std::string Trim(const std::string &s) {
  auto begin = s.begin();
  auto end = s.end();
  if (begin == end) {
    // Need to check this to be sure that std::prev(end) exists.
    return s;
  }
  while (begin < end && isspace(*begin)) {
    ++begin;
  }
  while (end > begin && isspace(*std::prev(end))) {
    --end;
  }
  return std::string(begin, end);
}

/**
 * Replaces all occurences of <match> in <src> with <replacement>.
 */
// TODO: This could be implemented much more efficiently.
inline std::string Replace(std::string src, const std::string &match,
                           const std::string &replacement) {
  for (size_t pos = src.find(match); pos != std::string::npos;
       pos = src.find(match, pos + replacement.size())) {
    src.erase(pos, match.length()).insert(pos, replacement);
  }
  return src;
}

/**
 * Outputs a collection of items to the given stream, separating them with the
 * given delimiter.
 *
 * @param stream Destination stream.
 * @param iterable An iterable collection of items.
 * @param delim Delimiter that is put between items.
 * @param streamer Function which accepts a TStream and an item and
 *  streams the item to the stream.
 */
template <typename TStream, typename TIterable>
inline void PrintIterable(TStream &stream, const TIterable &iterable,
                          const std::string &delim = ", ") {
  bool first = true;
  for (const auto &item : iterable) {
    if (first)
      first = false;
    else
      stream << delim;
    stream << item;
  }
}

/// Escapes all whitespace and quotation characters to produce a string
/// which can be used as a string literal.
inline std::string Escape(const std::string &src) {
  std::string ret;
  ret.reserve(src.size() + 2);
  ret.append(1, '"');
  for (auto c : src) {
    if (c == '\\' || c == '\'' || c == '"') {
      ret.append(1, '\\');
      ret.append(1, c);
    } else if (c == '\b') {
      ret.append("\\b");
    } else if (c == '\f') {
      ret.append("\\f");
    } else if (c == '\n') {
      ret.append("\\n");
    } else if (c == '\r') {
      ret.append("\\r");
    } else if (c == '\t') {
      ret.append("\\t");
    } else {
      ret.append(1, c);
    }
  }
  ret.append(1, '"');
  return ret;
}

}  // namespace utils

static void PrintValue(std::ostream &os, const mg_value *value);

static void PrintValue(std::ostream &os, const mg_string *str) {
  os << utils::Escape(std::string(mg_string_data(str), mg_string_size(str)));
}

static void PrintStringUnescaped(std::ostream &os, const mg_string *str) {
  os.write(mg_string_data(str), mg_string_size(str));
}

static void PrintValue(std::ostream &os, const mg_list *list) {
  os << "[";
  for (uint32_t i = 0; i < mg_list_size(list); ++i) {
    if (i > 0) {
      os << ", ";
    }
    PrintValue(os, mg_list_at(list, i));
  }
  os << "]";
}

static void PrintValue(std::ostream &os, const mg_map *map) {
  os << "{";
  for (uint32_t i = 0; i < mg_map_size(map); ++i) {
    if (i > 0) {
      os << ", ";
    }
    PrintStringUnescaped(os, mg_map_key_at(map, i));
    os << ": ";
    PrintValue(os, mg_map_value_at(map, i));
  }
  os << "}";
}

static void PrintValue(std::ostream &os, const mg_node *node) {
  os << "(";
  for (uint32_t i = 0; i < mg_node_label_count(node); ++i) {
    os << ":";
    PrintStringUnescaped(os, mg_node_label_at(node, i));
  }
  const mg_map *props = mg_node_properties(node);
  if (mg_node_label_count(node) > 0 && mg_map_size(props) > 0) {
    os << " ";
  }
  if (mg_map_size(props) > 0) {
    PrintValue(os, props);
  }
  os << ")";
}

static void PrintValue(std::ostream &os, const mg_relationship *rel) {
  os << "[:";
  PrintStringUnescaped(os, mg_relationship_type(rel));
  const mg_map *props = mg_relationship_properties(rel);
  if (mg_map_size(props) > 0) {
    os << " ";
    PrintValue(os, props);
  }
  os << "]";
}

static void PrintValue(std::ostream &os, const mg_unbound_relationship *rel) {
  os << "[:";
  PrintStringUnescaped(os, mg_unbound_relationship_type(rel));
  const mg_map *props = mg_unbound_relationship_properties(rel);
  if (mg_map_size(props) > 0) {
    os << " ";
    PrintValue(os, props);
  }
  os << "]";
}

static void PrintValue(std::ostream &os, const mg_path *path) {
  PrintValue(os, mg_path_node_at(path, 0));
  for (uint32_t i = 0; i < mg_path_length(path); ++i) {
    if (mg_path_relationship_reversed_at(path, i)) {
      os << "<-";
    } else {
      os << "-";
    }
    PrintValue(os, mg_path_relationship_at(path, i));
    if (mg_path_relationship_reversed_at(path, i)) {
      os << "-";
    } else {
      os << "->";
    }
    PrintValue(os, mg_path_node_at(path, i + 1));
  }
}

static void PrintValue(std::ostream &os, const mg_value *value) {
  switch (mg_value_get_type(value)) {
    case MG_VALUE_TYPE_NULL:
      os << "Null";
      return;
    case MG_VALUE_TYPE_BOOL:
      os << (mg_value_bool(value) ? "true" : "false");
      return;
    case MG_VALUE_TYPE_INTEGER:
      os << mg_value_integer(value);
      return;
    case MG_VALUE_TYPE_FLOAT:
      os << mg_value_float(value);
      return;
    case MG_VALUE_TYPE_STRING:
      PrintValue(os, mg_value_string(value));
      return;
    case MG_VALUE_TYPE_LIST:
      PrintValue(os, mg_value_list(value));
      return;
    case MG_VALUE_TYPE_MAP:
      PrintValue(os, mg_value_map(value));
      return;
    case MG_VALUE_TYPE_NODE:
      PrintValue(os, mg_value_node(value));
      return;
    case MG_VALUE_TYPE_RELATIONSHIP:
      PrintValue(os, mg_value_relationship(value));
      return;
    case MG_VALUE_TYPE_UNBOUND_RELATIONSHIP:
      PrintValue(os, mg_value_unbound_relationship(value));
      return;
    case MG_VALUE_TYPE_PATH:
      PrintValue(os, mg_value_path(value));
      return;
    default:
      os << "{unknown value}";
      break;
  }
}

static void EchoFailure(const std::string &failure_msg,
                        const std::string &explanation) {
  if (isatty(STDIN_FILENO)) {
    std::cout << "\033[1;31m" << failure_msg << ": \033[0m";
    std::cout << explanation << std::endl;
  } else {
    std::cerr << failure_msg << ": ";
    std::cerr << explanation << std::endl;
  }
}

static void EchoInfo(const std::string &message) {
  if (isatty(STDIN_FILENO)) {
    std::cout << message << std::endl;
  }
}

static void SetStdinEcho(bool enable = true) {
  struct termios tty;
  tcgetattr(STDIN_FILENO, &tty);
  if (!enable) {
    tty.c_lflag &= ~ECHO;
  } else {
    tty.c_lflag |= ECHO;
  }
  tcsetattr(STDIN_FILENO, TCSANOW, &tty);
}

/// Helper function that sets default input for 'readline'
static int SetDefaultText() {
  rl_insert_text(default_text.c_str());
  default_text = "";
  rl_startup_hook = (rl_hook_func_t *)NULL;
  return 0;
}

/// Memgraph and OpenCypher keywords.
static const std::vector<std::string> kMemgraphKeywords{
    "ALTER",    "AUTH",    "BATCH", "BATCHES", "CLEAR",     "DATA",
    "DENY",     "DROP",    "FOR",   "FROM",    "GRANT",     "IDENTIFIED",
    "INTERVAL", "K_TEST",  "KAFKA", "LOAD",    "PASSWORD",  "PRIVILEGES",
    "REVOKE",   "ROLE",    "ROLES", "SIZE",    "START",     "STOP",
    "STREAM",   "STREAMS", "TO",    "TOPIC",   "TRANSFORM", "USER",
    "USERS"};
static const std::vector<std::string> kCypherKeywords{
    "ALL",        "AND",    "ANY",    "AS",         "ASC",      "ASCENDING",
    "BFS",        "BY",     "CASE",   "CONTAINS",   "COUNT",    "CREATE",
    "CYPHERNULL", "DELETE", "DESC",   "DESCENDING", "DETACH",   "DISTINCT",
    "ELSE",       "END",    "ENDS",   "EXTRACT",    "FALSE",    "FILTER",
    "IN",         "INDEX",  "IS",     "LIMIT",      "L_SKIP",   "MATCH",
    "MERGE",      "NONE",   "NOT",    "ON",         "OPTIONAL", "OR",
    "ORDER",      "REDUCE", "REMOVE", "RETURN",     "SET",      "SHOW",
    "SINGLE",     "STARTS", "THEN",   "TRUE",       "UNION",    "UNWIND",
    "WHEN",       "WHERE",  "WITH",   "WSHORTEST",  "XOR"};

static char *CompletionGenerator(const char *text, int state) {
  // This function is called with state=0 the first time; subsequent calls
  // are with a nonzero state. state=0 can be used to perform one-time
  // initialization for this completion session.
  static std::vector<std::string> matches;
  static size_t match_index = 0;

  if (state == 0) {
    // During initialization, compute the actual matches for 'text' and
    // keep them in a static vector.
    matches.clear();
    match_index = 0;

    // Collect a vector of matches: vocabulary words that begin with text.
    std::string text_str = utils::ToUpperCase(std::string(text));
    for (auto word : kCypherKeywords) {
      if (word.size() >= text_str.size() &&
          word.compare(0, text_str.size(), text_str) == 0) {
        matches.push_back(word);
      }
    }
    for (auto word : kMemgraphKeywords) {
      if (word.size() >= text_str.size() &&
          word.compare(0, text_str.size(), text_str) == 0) {
        matches.push_back(word);
      }
    }
  }

  if (match_index >= matches.size()) {
    // We return nullptr to notify the caller no more matches are available.
    return nullptr;
  } else {
    // Return a malloc'd char* for the match. The caller frees it.
    return strdup(matches[match_index++].c_str());
  }
}

static char **Completer(const char *text, int, int) {
  // Don't do filename completion even if our generator finds no matches.
  rl_attempted_completion_over = 1;
  // Note: returning nullptr here will make readline use the default filename
  // completer. This note is copied from examples - I think because
  // rl_attempted_completion_over is set to 1, filename completer won't be
  // used.
  return rl_completion_matches(text, CompletionGenerator);
}

/// Helper function that reads a line from the
/// standard input using the 'readline' lib.
/// Adds support for history and reverse-search.
///
/// @param prompt The prompt to display.
/// @return  User input line, or nullopt on EOF.
static std::experimental::optional<std::string> ReadLine(
    const std::string &prompt) {
  if (default_text.size() > 0) {
    // Initialize text with remainder of previous query.
    rl_startup_hook = SetDefaultText;
  }
  char *line = readline(prompt.c_str());
  if (!line) return std::experimental::nullopt;

  std::string r_val(line);
  if (!utils::Trim(r_val).empty()) add_history(line);
  free(line);
  return r_val;
}

static std::experimental::optional<std::string> GetLine() {
  std::string line;
  std::getline(std::cin, line);
  if (std::cin.eof()) return std::experimental::nullopt;
  line = default_text + line;
  default_text = "";
  return line;
}

/// Helper function that parses user line input.
/// @param line user input line.
/// @param quote quote character or '\0'; if set line is inside quotation.
/// @param escaped if set, next character should be escaped.
/// @return pair of string and bool. string is parsed line and bool marks
/// if query finished(Query finishes with ';') with this line.
static std::pair<std::string, bool> ParseLine(const std::string &line,
                                              char *quote, bool *escaped) {
  // Parse line.
  bool is_done = false;
  std::stringstream parsed_line;
  for (auto c : line) {
    if (*quote && c == '\\') {
      // Escaping is only used inside quotation to not end the quote
      // when quotation char is escaped.
      *escaped = !*escaped;
      parsed_line << c;
      continue;
    } else if ((!*quote && (c == '\"' || c == '\'')) ||
               (!*escaped && c == *quote)) {
      *quote = *quote ? '\0' : c;
    } else if (!*quote && c == ';') {
      is_done = true;
      break;
    }
    parsed_line << c;
    *escaped = false;
  }
  return std::make_pair(parsed_line.str(), is_done);
}

static std::experimental::optional<std::string> GetQuery() {
  char quote = '\0';
  bool escaped = false;
  auto ret = ParseLine(default_text, &quote, &escaped);
  if (ret.second) {
    auto idx = ret.first.size() + 1;
    default_text = utils::Trim(default_text.substr(idx));
    return ret.first;
  }
  std::stringstream query;
  std::experimental::optional<std::string> line;
  int line_cnt = 0;
  auto is_done = false;
  while (!is_done) {
    if (!isatty(STDIN_FILENO)) {
      line = GetLine();
    } else {
      line = ReadLine(line_cnt == 0 ? kPrompt : kMultilinePrompt);
      if (line_cnt == 0 && line && line->size() > 0 && (*line)[0] == ':') {
        auto trimmed_line = utils::Trim(*line);
        if (trimmed_line == kCommandQuit) {
          return std::experimental::nullopt;
        } else if (trimmed_line == kCommandHelp) {
          PrintHelp();
          return "";
        } else {
          EchoFailure("Unsupported command", trimmed_line);
          PrintHelp();
          return "";
        }
      }
    }
    if (!line) return std::experimental::nullopt;
    if (line->empty()) continue;
    auto ret = ParseLine(*line, &quote, &escaped);
    query << ret.first;
    auto char_count = ret.first.size();
    if (ret.second) {
      is_done = true;
      char_count += 1;  // ';' sign
    } else {
      // Query is multiline so append newline.
      query << "\n";
    }
    if (char_count < line->size()) {
      default_text = utils::Trim(line->substr(char_count));
    }
    ++line_cnt;
  }
  return query.str();
}

static void PrintRowTabular(const wrap::MgListPtr &data, int total_width,
                            int column_width, int num_columns,
                            bool all_columns_fit, int margin = 1) {
  if (!all_columns_fit) num_columns -= 1;
  std::string data_output = std::string(total_width, ' ');
  for (auto i = 0; i < total_width; i += column_width) {
    data_output[i] = '|';
    int idx = i / column_width;
    if (idx < num_columns) {
      std::stringstream field;
      PrintValue(field,
                 mg_list_at(data.get(), idx));  // convert Value to string
      std::string field_str(field.str());
      if ((int)field_str.size() > column_width - 2 * margin - 1) {
        field_str.erase(column_width - 2 * margin - 1, std::string::npos);
        field_str.replace(field_str.size() - 3, 3, "...");
      }
      data_output.replace(i + 1 + margin, field_str.size(), field_str);
    }
  }
  if (!all_columns_fit) {
    data_output.replace(total_width - column_width, 3, "...");
  }
  data_output[total_width - 1] = '|';
  std::cout << data_output << std::endl;
}

static void PrintHeaderTabular(const std::vector<std::string> &data,
                               int total_width, int column_width,
                               int num_columns, bool all_columns_fit,
                               int margin = 1) {
  if (!all_columns_fit) num_columns -= 1;
  std::string data_output = std::string(total_width, ' ');
  for (auto i = 0; i < total_width; i += column_width) {
    data_output[i] = '|';
    int idx = i / column_width;
    if (idx < num_columns) {
      std::string field_str = data[idx];
      if ((int)field_str.size() > column_width - 2 * margin - 1) {
        field_str.erase(column_width - 2 * margin - 1, std::string::npos);
        field_str.replace(field_str.size() - 3, 3, "...");
      }
      data_output.replace(i + 1 + margin, field_str.size(), field_str);
    }
  }
  if (!all_columns_fit) {
    data_output.replace(total_width - column_width, 3, "...");
  }
  data_output[total_width - 1] = '|';
  std::cout << data_output << std::endl;
}

/// Helper function for determining maximum length of data.
/// @param data List of mg_values representing row.
/// @param margin Column margin width.
/// @return length needed for representing max size element in @p data list.
/// Plus one is added because of column start character '|'.
static uint64_t GetMaxColumnWidth(const wrap::MgListPtr &data, int margin = 1) {
  uint64_t column_width = 0;
  for (uint32_t i = 0; i < mg_list_size(data.get()); ++i) {
    std::stringstream field;
    PrintValue(field, mg_list_at(data.get(), i));
    column_width = std::max(column_width, field.str().size() + 2 * margin);
  }
  return column_width + 1;
}

static uint64_t GetMaxColumnWidth(const std::vector<std::string> &data,
                                  int margin = 1) {
  uint64_t column_width = 0;
  for (const auto &field : data) {
    column_width = std::max(column_width, field.size() + 2 * margin);
  }
  return column_width + 1;
}

static void PrintTabular(const std::vector<std::string> &header,
                         const std::vector<wrap::MgListPtr> &records) {
  struct winsize w;
  ioctl(STDOUT_FILENO, TIOCGWINSZ, &w);
  bool all_columns_fit = true;

  auto num_columns = header.size();
  auto column_width = GetMaxColumnWidth(header);
  for (size_t i = 0; i < records.size(); ++i) {
    column_width = std::max(column_width, GetMaxColumnWidth(records[i]));
  }
  column_width = std::max(static_cast<uint64_t>(5),
                          column_width);  // set column width to min 5
  auto total_width = column_width * num_columns + 1;

  // Fit to screen width.
  if (FLAGS_fit_to_screen && total_width > w.ws_col) {
    uint64_t lo = 5;
    uint64_t hi = column_width;
    uint64_t last = 5;
    while (lo < hi) {
      uint64_t mid = lo + (hi - lo) / 2;
      uint64_t width = mid * num_columns + 1;
      if (width <= w.ws_col) {
        last = mid;
        lo = mid + 1;
      } else {
        hi = mid - 1;
      }
    }
    column_width = last;
    total_width = column_width * num_columns + 1;
    // All columns do not fit on screen.
    while (total_width > w.ws_col && num_columns > 1) {
      num_columns -= 1;
      total_width = column_width * num_columns + 1;
      all_columns_fit = false;
    }
  }

  auto line_fill = std::string(total_width, '-');
  for (auto i = 0u; i < total_width; i += column_width) {
    line_fill[i] = '+';
  }
  line_fill[total_width - 1] = '+';
  std::cout << line_fill << std::endl;
  // Print Header.
  PrintHeaderTabular(header, total_width, column_width, num_columns,
                     all_columns_fit);
  std::cout << line_fill << std::endl;
  // Print Records.
  for (size_t i = 0; i < records.size(); ++i) {
    PrintRowTabular(records[i], total_width, column_width, num_columns,
                    all_columns_fit);
  }
  std::cout << line_fill << std::endl;
}

static std::vector<std::string> FormatCsvFields(const wrap::MgListPtr &fields) {
  std::vector<std::string> formatted;
  formatted.reserve(mg_list_size(fields.get()));
  for (uint32_t i = 0; i < mg_list_size(fields.get()); ++i) {
    std::stringstream field_stream;
    PrintValue(field_stream, mg_list_at(fields.get(), i));
    std::string formatted_field(field_stream.str());
    if (FLAGS_csv_doublequote) {
      formatted_field = utils::Replace(formatted_field, "\"", "\"\"");
    } else {
      formatted_field =
          utils::Replace(formatted_field, "\"", FLAGS_csv_escapechar + "\"");
    }
    formatted_field.insert(0, 1, '"');
    formatted_field.append(1, '"');
    formatted.push_back(formatted_field);
  }
  return formatted;
}

static std::vector<std::string> FormatCsvHeader(
    const std::vector<std::string> &fields) {
  std::vector<std::string> formatted;
  formatted.reserve(fields.size());
  for (auto formatted_field : fields) {
    if (FLAGS_csv_doublequote) {
      formatted_field = utils::Replace(formatted_field, "\"", "\"\"");
    } else {
      formatted_field =
          utils::Replace(formatted_field, "\"", FLAGS_csv_escapechar + "\"");
    }
    formatted_field.insert(0, 1, '"');
    formatted_field.append(1, '"');
    formatted.push_back(formatted_field);
  }
  return formatted;
}

static void PrintCsv(const std::vector<std::string> &header,
                     const std::vector<wrap::MgListPtr> &records) {
  // Print Header.
  auto formatted_header = FormatCsvHeader(header);
  utils::PrintIterable(std::cout, formatted_header, FLAGS_csv_delimiter);
  std::cout << std::endl;
  // Print Records.
  for (size_t i = 0; i < records.size(); ++i) {
    auto formatted_row = FormatCsvFields(records[i]);
    utils::PrintIterable(std::cout, formatted_row, FLAGS_csv_delimiter);
    std::cout << std::endl;
  }
}

static void Output(const std::vector<std::string> &header,
                   const std::vector<wrap::MgListPtr> &records) {
  if (FLAGS_output_format == kTabularFormat) {
    PrintTabular(header, records);
  } else if (FLAGS_output_format == kCsvFormat) {
    PrintCsv(header, records);
  }
}

struct QueryData {
  std::vector<std::string> header;
  std::vector<wrap::MgListPtr> records;
  std::chrono::duration<double> wall_time;
};

QueryData ExecuteQuery(mg_session *session, const std::string &query) {
  int status = mg_session_run(session, query.c_str(), nullptr, nullptr, nullptr, nullptr);
  auto start = std::chrono::system_clock::now();
  if (status != 0) {
    if (mg_session_status(session) == MG_SESSION_BAD) {
      throw utils::ClientFatalException(mg_session_error(session));
    } else {
      throw utils::ClientQueryException(mg_session_error(session));
    }
  }

  status = mg_session_pull(session, nullptr);
  if (status != 0) {
    if (mg_session_status(session) == MG_SESSION_BAD) {
      throw utils::ClientFatalException(mg_session_error(session));
    } else {
      throw utils::ClientQueryException(mg_session_error(session));
    }
  }

  QueryData ret;
  mg_result *result;
  while ((status = mg_session_fetch(session, &result)) == 1) {
    ret.records.push_back(
        wrap::MakeCustomUnique<mg_list>(mg_list_copy(mg_result_row(result))));
    if (!ret.records.back()) {
      std::cerr << "out of memory";
      std::abort();
    }
  }
  if (status != 0) {
    if (mg_session_status(session) == MG_SESSION_BAD) {
      throw utils::ClientFatalException(mg_session_error(session));
    } else {
      throw utils::ClientQueryException(mg_session_error(session));
    }
  }

  {
    const mg_list *header = mg_result_columns(result);
    for (uint32_t i = 0; i < mg_list_size(header); ++i) {
      const mg_value *field = mg_list_at(header, i);
      if (mg_value_get_type(field) == MG_VALUE_TYPE_STRING) {
        ret.header.push_back(
            std::string(mg_string_data(mg_value_string(field)),
                        mg_string_size(mg_value_string(field))));
      } else {
        std::stringstream field_stream;
        PrintValue(field_stream, field);
        ret.header.push_back(field_stream.str());
      }
    }
  }

  ret.wall_time = std::chrono::system_clock::now() - start;
  return ret;
}

int main(int argc, char **argv) {
  gflags::SetVersionString(version_string);
  gflags::SetUsageMessage(kUsage);

  gflags::ParseCommandLineFlags(&argc, &argv, true);
  if (FLAGS_output_format == kCsvFormat && !ValidateCsvDoubleQuote()) {
    EchoFailure(
        "Unsupported combination of 'csv-doublequote' and 'csv-escapechar'\n"
        "flags",
        "Run '" + std::string(argv[0]) + " --help' for usage.");
    return 1;
  }
  auto password = FLAGS_password;
  if (isatty(STDIN_FILENO) && FLAGS_username.size() > 0 &&
      password.size() == 0) {
    SetStdinEcho(false);
    auto password_optional = ReadLine("Password: ");
    std::cout << std::endl;
    if (password_optional) {
      password = *password_optional;
    } else {
      EchoFailure("Password not submitted",
                  "Requested password for username " + FLAGS_username);
      return 1;
    }
    SetStdinEcho(true);
  }

  using_history();
  int history_len = 0;
  rl_attempted_completion_function = Completer;
  fs::path history_dir = FLAGS_history;
  if (FLAGS_history ==
      (kDefaultHistoryBaseDir + "/" + kDefaultHistoryMemgraphDir)) {
    // Fetch home dir for user.
    struct passwd *pw = getpwuid(getuid());
    history_dir = fs::path(pw->pw_dir) / kDefaultHistoryMemgraphDir;
  }
  if (!utils::EnsureDir(history_dir)) {
    EchoFailure("History directory doesn't exist", history_dir);
    // Should program exit here or just continue with warning message?
    return 1;
  }
  fs::path history_file = history_dir / kHistoryFilename;
  // Read history file.
  if (fs::exists(history_file)) {
    auto ret = read_history(history_file.string().c_str());
    if (ret != 0) {
      EchoFailure("Unable to read history file", history_file);
      // Should program exit here or just continue with warning message?
      return 1;
    }
    history_len = history_length;
  }

  // Save history function. Used to save readline history after each query.
  auto save_history = [&history_len, history_file] {
    if (!FLAGS_no_history) {
      int ret = 0;
      // If there was no history, create history file.
      // Otherwise, append to existing history.
      if (history_len == 0) {
        ret = write_history(history_file.string().c_str());
      } else {
        ret = append_history(1, history_file.string().c_str());
      }
      if (ret != 0) {
        EchoFailure("Unable to save history to file", history_file);
        return 1;
      }
      ++history_len;
    }
    return 0;
  };

  auto shutdown = [](int exit_code = 0) {
    if (is_shutting_down) return;
    is_shutting_down = 1;
    std::quick_exit(exit_code);
  };

  struct sigaction action;
  action.sa_sigaction = nullptr;
  action.sa_handler = shutdown;
  // Prevent handling shutdown inside a shutdown. For example, SIGINT handler
  // being interrupted by SIGTERM before is_shutting_down is set, thus causing
  // double shutdown.
  sigemptyset(&action.sa_mask);
  sigaddset(&action.sa_mask, SIGTERM);
  sigaddset(&action.sa_mask, SIGINT);
  action.sa_flags = SA_RESTART;
  sigaction(SIGTERM, &action, nullptr);
  sigaction(SIGINT, &action, nullptr);

  std::string bolt_client_version = "mg/"s + gflags::VersionString();

  wrap::MgSessionParamsPtr params =
      wrap::MakeCustomUnique<mg_session_params>(mg_session_params_make());
  if (!params) {
    EchoFailure("Connection failure",
                "out of memory, failed to allocate `mg_session_params` struct");
  }
  mg_session_params_set_host(params.get(), FLAGS_host.c_str());
  mg_session_params_set_port(params.get(), FLAGS_port);
  if (!FLAGS_username.empty()) {
    mg_session_params_set_username(params.get(), FLAGS_username.c_str());
    mg_session_params_set_password(params.get(), password.c_str());
  }
  mg_session_params_set_user_agent(params.get(), bolt_client_version.c_str());
  mg_session_params_set_sslmode(
      params.get(), FLAGS_use_ssl ? MG_SSLMODE_REQUIRE : MG_SSLMODE_DISABLE);

  wrap::MgSessionPtr session = wrap::MakeCustomUnique<mg_session>(nullptr);
  {
    mg_session *session_tmp;
    int status = mg_connect(params.get(), &session_tmp);
    session = wrap::MakeCustomUnique<mg_session>(session_tmp);
    if (status != 0) {
      EchoFailure("Connection failure", mg_session_error(session.get()));
      return 1;
    }
  }

  EchoInfo("mgconsole "s + gflags::VersionString());
  EchoInfo("Type :help for shell usage");
  EchoInfo("Quit the shell by typing Ctrl-D(eof) or :quit");
  EchoInfo("Connected to 'memgraph://" + FLAGS_host + ":" +
           std::to_string(FLAGS_port) + "'");
  int num_retries = 3;
  while (true) {
    auto query = GetQuery();
    if (!query) {
      EchoInfo("Bye");
      break;
    }
    if (query->empty()) continue;
    try {
      auto ret = ExecuteQuery(session.get(), *query);
      if (ret.records.size() > 0) Output(ret.header, ret.records);
      if (isatty(STDIN_FILENO)) {
        std::string summary;
        if (ret.records.size() == 0) {
          summary = "Empty set";
        } else if (ret.records.size() == 1) {
          summary = std::to_string(ret.records.size()) + " row in set";
        } else {
          summary = std::to_string(ret.records.size()) + " rows in set";
        }
        std::printf("%s (%.3lf sec)\n", summary.c_str(), ret.wall_time.count());
        auto history_ret = save_history();
        if (history_ret != 0) return history_ret;
      }
    } catch (const utils::ClientQueryException &e) {
      if (!isatty(STDIN_FILENO)) {
        EchoFailure("Failed query", *query);
      }
      EchoFailure("Client received exception", e.what());
      if (!isatty(STDIN_FILENO)) {
        return 1;
      }
    } catch (const utils::ClientFatalException &e) {
      EchoFailure("Client received exception", e.what());
      EchoInfo("Trying to reconnect");
      bool is_connected = false;
      session.reset(nullptr);
      while (num_retries > 0) {
        --num_retries;
        mg_session *session_tmp;
        int status = mg_connect(params.get(), &session_tmp);
        session = wrap::MakeCustomUnique<mg_session>(session_tmp);
        if (status != 0) {
          EchoFailure("Connection failure", mg_session_error(session.get()));
          session.reset(nullptr);
        } else {
          is_connected = true;
          break;
        }
        std::this_thread::sleep_for(std::chrono::seconds(1));
      }
      if (is_connected) {
        num_retries = 3;
        EchoInfo("Connected to 'memgraph://" + FLAGS_host + ":" +
                 std::to_string(FLAGS_port) + "'");
      } else {
        EchoFailure("Couldn't connect to", "'memgraph://" + FLAGS_host + ":" +
                                               std::to_string(FLAGS_port) +
                                               "'");
        return 1;
      }
    }
  }
  return 0;
}
