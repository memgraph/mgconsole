#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <ios>
#include <iostream>
#include <string.h>
#include <string>
#include <string_view>
#include <type_traits>
#include <unordered_map>

#ifdef __APPLE__

#include <sstream>

#endif /* __APPLE__ */

#ifdef _WIN32

#include <io.h>
#include <stdlib.h>
#include <windows.h>
#define isatty _isatty

#else /* _WIN32 */

#include <pwd.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

#endif /* _WIN32 */

#include <gflags/gflags.h>
#include <replxx.h>

#include "constants.hpp"
#include "date.hpp"
#include "mgclient.h"
#include "utils.hpp"

namespace utils {

bool EnsureDir(const fs::path &dir) noexcept {
  std::error_code error_code; // for exception suppression.
  if (fs::exists(dir, error_code))
    return fs::is_directory(dir, error_code);
  return fs::create_directories(dir, error_code);
}

fs::path GetUserHomeDir() {
#ifdef _WIN32
  return getenv("USERPROFILE");
#else  /* _WIN32 */
  struct passwd *pw = getpwuid(getuid());
  return pw->pw_dir;
#endif /* _WIN32 */
}

std::string ToUpperCase(std::string s) {
  std::transform(s.begin(), s.end(), s.begin(),
                 [](char c) { return toupper(c); });
  return s;
}

std::string Trim(const std::string &s) {
  auto begin = s.begin();
  auto end = s.end();
  if (begin == end) {
    // need to check this to be sure that std::prev(end) exists.
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

std::string Replace(std::string src, const std::string &match,
                    const std::string &replacement) {
  for (size_t pos = src.find(match); pos != std::string::npos;
       pos = src.find(match, pos + replacement.size())) {
    src.erase(pos, match.length()).insert(pos, replacement);
  }
  return src;
}

std::string Escape(const std::string &src) {
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

template <typename T, std::enable_if_t<std::is_arithmetic<T>::value, T> = true>
void PrintIfNotZero(std::ostream &os, T value, std::string_view suffix = "") {
  if (value) {
    os << value << suffix;
  }
}

void PrintStringUnescaped(std::ostream &os, const mg_string *str) {
  os.write(mg_string_data(str), mg_string_size(str));
}

void PrintValue(std::ostream &os, const mg_string *str) {
  os << utils::Escape(std::string(mg_string_data(str), mg_string_size(str)));
}

void PrintValue(std::ostream &os, const mg_list *list) {
  os << "[";
  for (uint32_t i = 0; i < mg_list_size(list); ++i) {
    if (i > 0) {
      os << ", ";
    }
    PrintValue(os, mg_list_at(list, i));
  }
  os << "]";
}

void PrintValue(std::ostream &os, const mg_map *map) {
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

void PrintValue(std::ostream &os, const mg_node *node) {
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

void PrintValue(std::ostream &os, const mg_relationship *rel) {
  os << "[:";
  PrintStringUnescaped(os, mg_relationship_type(rel));
  const mg_map *props = mg_relationship_properties(rel);
  if (mg_map_size(props) > 0) {
    os << " ";
    PrintValue(os, props);
  }
  os << "]";
}

void PrintValue(std::ostream &os, const mg_unbound_relationship *rel) {
  os << "[:";
  PrintStringUnescaped(os, mg_unbound_relationship_type(rel));
  const mg_map *props = mg_unbound_relationship_properties(rel);
  if (mg_map_size(props) > 0) {
    os << " ";
    PrintValue(os, props);
  }
  os << "]";
}

void PrintValue(std::ostream &os, const mg_path *path) {
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

void PrintValue(std::ostream &os, const mg_date *date) {
  const auto days = date::days(mg_date_days(date));
  const auto sys_days = date::sys_days(days);
  const auto ymd = date::year_month_day(sys_days);
  os << ymd;
}

void PrintValue(std::ostream &os, const mg_local_time *local_time) {
  namespace chrono = std::chrono;
  const auto nanos = chrono::nanoseconds(mg_local_time_nanoseconds(local_time));
  const auto hms = date::hh_mm_ss(nanos);
  os << hms;
}

void PrintValue(std::ostream &os, const mg_local_date_time *local_date_time) {
  namespace chrono = std::chrono;
  const auto seconds =
      chrono::seconds(mg_local_date_time_seconds(local_date_time));
  const auto days = chrono::duration_cast<date::days>(seconds);
  const auto date = mg_date_make(days.count());

  const auto nanoseconds = chrono::duration_cast<chrono::nanoseconds>(seconds) -
                           chrono::duration_cast<chrono::nanoseconds>(days);
  const auto time = mg_local_time_make(nanoseconds.count());

  PrintValue(os, date);
  os << " ";
  PrintValue(os, time);
}

void PrintSeconds(std::ostream &os, std::chrono::seconds ss,
                  std::chrono::microseconds mis) {
  if (ss.count() == 0 && mis.count() < 0) {
    os << '-';
  }
  os << ss.count();
}

void PrintMicroseconds(std::ostream &os, std::chrono::microseconds mis) {
  if (mis.count() != 0) {
    os << "." << std::setw(6) << std::setfill('0') << std::fixed
       << std::abs(mis.count());
  }
}

void PrintValue(std::ostream &os, const mg_duration *duration) {
  // Currently we are ignoring months for duration
  // const auto months = date::months(mg_duration_months(duration));
  namespace chrono = std::chrono;
  const auto days = date::days(mg_duration_days(duration));
  const auto seconds = chrono::seconds(mg_duration_seconds(duration));
  const auto nanoseconds =
      chrono::nanoseconds(mg_duration_nanoseconds(duration));

  const auto time =
      chrono::duration_cast<chrono::microseconds>(seconds + nanoseconds);

  const bool has_subdays = time.count() > 0;

  const auto hh = chrono::duration_cast<chrono::hours>(time);
  const auto mm = chrono::duration_cast<chrono::minutes>(time - hh);
  const auto ss = chrono::duration_cast<chrono::seconds>(time - hh - mm);
  const auto mis =
      chrono::duration_cast<chrono::microseconds>(time - hh - mm - ss);

  os << "P";
  PrintIfNotZero(os, days.count(), "D");

  if (has_subdays) {
    os << "T";
  }

  PrintIfNotZero(os, hh.count(), "H");
  PrintIfNotZero(os, mm.count(), "M");
  if (ss.count() == 0 && mis.count() == 0) {
    return;
  }
  PrintSeconds(os, ss, mis);
  PrintMicroseconds(os, mis);
  os << "S";
}

void PrintValue(std::ostream &os, const mg_value *value) {
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
  case MG_VALUE_TYPE_DATE:
    PrintValue(os, mg_value_date(value));
    return;
  case MG_VALUE_TYPE_LOCAL_TIME:
    PrintValue(os, mg_value_local_time(value));
    return;
  case MG_VALUE_TYPE_LOCAL_DATE_TIME:
    PrintValue(os, mg_value_local_date_time(value));
    return;
  case MG_VALUE_TYPE_DURATION:
    PrintValue(os, mg_value_duration(value));
    return;
  default:
    os << "{unknown value}";
    break;
  }
}

} // namespace utils

namespace console {

// lifted from replxx io.cxx
bool is_a_tty(int fd) {
  bool aTTY(isatty(fd) != 0);
#ifdef _WIN32
  do {
    if (aTTY) {
      break;
    }
    HANDLE h((HANDLE)_get_osfhandle(fd));
    if (h == INVALID_HANDLE_VALUE) {
      break;
    }
    DWORD st(0);
    if (!GetConsoleMode(h, &st)) {
      break;
    }
    aTTY = true;
  } while (false);
#endif
  return (aTTY);
}

void PrintHelp() { std::cout << constants::kInteractiveUsage << std::endl; }

void PrintDocs() { std::cout << constants::kDocs << std::endl; }

void EchoFailure(const std::string &failure_msg,
                 const std::string &explanation) {
  if (is_a_tty(STDIN_FILENO)) {
#ifdef _WIN32
    HANDLE hConsole;
    WORD original_console_attr;
    CONSOLE_SCREEN_BUFFER_INFO csbi;

    hConsole = GetStdHandle(STD_OUTPUT_HANDLE);
    if (!GetConsoleScreenBufferInfo(hConsole, &csbi)) {
      std::cout << failure_msg << ": ";
    }

    original_console_attr = csbi.wAttributes;
    FlushConsoleInputBuffer(hConsole);
    SetConsoleTextAttribute(hConsole, FOREGROUND_RED | FOREGROUND_INTENSITY);

    std::cout << failure_msg << ":";
    SetConsoleTextAttribute(hConsole, original_console_attr);
    std::cout << " ";
#else  /* _WIN32 */
    std::cout << "\033[1;31m" << failure_msg << ": \033[0m";
#endif /* _WIN32 */
    std::cout << explanation << std::endl;
  } else {
    std::cerr << failure_msg << ": ";
    std::cerr << explanation << std::endl;
  }
}

void EchoInfo(const std::string &message) {
  if (is_a_tty(STDIN_FILENO)) {
    std::cout << message << std::endl;
  }
}

void EchoStats(const std::map<std::string, int64_t> &stats) {
  for (const auto &[key, value] : stats) {
    if (value == 0) {
      continue;
    }
    if (key == "nodes-created") {
      std::printf("%ld vertices have been created.\n", value);
    }
    if (key == "nodes-deleted") {
      std::printf("%ld vertices have been deleted.\n", value);
    }
    if (key == "relationships-created") {
      std::printf("%ld edges have been created.\n", value);
    }
    if (key == "relationships-deleted") {
      std::printf("%ld edges have been deleted.\n", value);
    }
    if (key == "labels-added") {
      std::printf("%ld labels have been created.\n", value);
    }
    if (key == "labels-removed") {
      std::printf("%ld labels have been deleted.\n", value);
    }
    if (key == "properties-set") {
      std::printf("%ld properties have been updated.\n", value);
    }
  }
}

void EchoNotification(const std::map<std::string, std::string> &notification) {
  std::printf("%s: %s\n", notification.at("severity").c_str(),
              notification.at("code").c_str());
}

void SetStdinEcho(bool enable = true) {
#ifdef _WIN32
  // from
  // https://stackoverflow.com/questions/9217908/how-to-disable-echo-in-windows-console
  HANDLE h;
  DWORD mode;
  h = GetStdHandle(STD_INPUT_HANDLE);
  if (GetConsoleMode(h, &mode)) {
    if (!enable) {
      mode &= ~ENABLE_ECHO_INPUT;
    } else {
      mode |= ENABLE_ECHO_INPUT;
    }
    SetConsoleMode(h, mode);
  }
#else  /* _WIN32 */
  struct termios tty;
  tcgetattr(STDIN_FILENO, &tty);
  if (!enable) {
    tty.c_lflag &= ~ECHO;
  } else {
    tty.c_lflag |= ECHO;
  }
  tcsetattr(STDIN_FILENO, TCSANOW, &tty);
#endif /* _WIN32 */
}

std::optional<std::string> GetLine() {
  std::string line;
  std::getline(std::cin, line);
  if (std::cin.eof())
    return std::nullopt;
  line = default_text + line;
  default_text = "";
  return line;
}

std::pair<std::string, bool> ParseLine(const std::string &line, char *quote,
                                       bool *escaped) {
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

std::optional<std::string> ReadLine(Replxx *replxx_instance,
                                    const std::string &prompt) {
  if (!default_text.empty()) {
    replxx_set_preload_buffer(replxx_instance, default_text.c_str());
  }
  const char *line = replxx_input(replxx_instance, prompt.c_str());
  default_text = "";
  if (!line) {
    return std::nullopt;
  }

  std::string r_val(line);
  if (!utils::Trim(r_val).empty())
    replxx_history_add(replxx_instance, line);
  return r_val;
}

} // namespace console

namespace query {

std::optional<std::string> GetQuery(Replxx *replxx_instance) {
  char quote = '\0';
  bool escaped = false;
  auto ret = console::ParseLine(default_text, &quote, &escaped);
  if (ret.second) {
    auto idx = ret.first.size() + 1;
    default_text = utils::Trim(default_text.substr(idx));
    return ret.first;
  }
  std::stringstream query;
  std::optional<std::string> line;
  int line_cnt = 0;
  auto is_done = false;
  while (!is_done) {
    if (!console::is_a_tty(STDIN_FILENO)) {
      line = console::GetLine();
    } else {
      line = console::ReadLine(replxx_instance,
                               line_cnt == 0 ? constants::kPrompt
                                             : constants::kMultilinePrompt);
      if (line_cnt == 0 && line && line->size() > 0 && (*line)[0] == ':') {
        auto trimmed_line = utils::Trim(*line);
        if (trimmed_line == constants::kCommandQuit) {
          return std::nullopt;
        } else if (trimmed_line == constants::kCommandHelp) {
          console::PrintHelp();
          return "";
        } else if (trimmed_line == constants::kCommandDocs) {
          console::PrintDocs();
          return "";
        } else {
          console::EchoFailure("Unsupported command", trimmed_line);
          console::PrintHelp();
          return "";
        }
      }
    }
    if (!line)
      return std::nullopt;
    if (line->empty())
      continue;
    auto ret = console::ParseLine(*line, &quote, &escaped);
    query << ret.first;
    auto char_count = ret.first.size();
    if (ret.second) {
      is_done = true;
      char_count += 1; // ';' sign
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

QueryData ExecuteQuery(mg_session *session, const std::string &query) {
  int status = mg_session_run(session, query.c_str(), nullptr, nullptr, nullptr,
                              nullptr);
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
    ret.records.push_back(mg_memory::MakeCustomUnique<mg_list>(
        mg_list_copy(mg_result_row(result))));
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
        utils::PrintValue(field_stream, field);
        ret.header.push_back(field_stream.str());
      }
    }
  }

  {
    const mg_map *summary = mg_result_summary(result);
    for (size_t i = 0; i < mg_map_size(summary); i++) {
      const mg_string *key = mg_map_key_at(summary, i);

      // Parse only stats and notifications from summary
      if (strcmp(mg_string_data(key), "stats") == 0) {
        ret.stats = std::map<std::string, int64_t>{};
        const mg_map *stats_map = mg_value_map(mg_map_value_at(summary, i));

        for (size_t j = 0; j < mg_map_size(stats_map); j++) {
          const auto *stat_key = mg_string_data(mg_map_key_at(stats_map, j));

          const int64_t stat_value =
              mg_value_integer(mg_map_value_at(stats_map, j));
          ret.stats->insert({stat_key, stat_value});
        }
      } else if (strcmp(mg_string_data(key), "notifications") == 0) {
        ret.notification = std::map<std::string, std::string>{};
        const mg_list *notifications =
            mg_value_list(mg_map_value_at(summary, i));
        if (mg_list_size(notifications) == 0) {
          continue;
        }
        // For now support only one notification
        const mg_map *notification_map =
            mg_value_map(mg_list_at(notifications, 0));

        for (size_t j = 0; j < mg_map_size(notification_map); j++) {
          const auto *notification_key =
              mg_string_data(mg_map_key_at(notification_map, j));

          const mg_string *mg_notification_value =
              mg_value_string(mg_map_value_at(notification_map, j));
          const auto *notification_value =
              mg_string_data(mg_notification_value);
          ret.notification->insert({notification_key, notification_value});
        }
      }
    }
  }

  ret.wall_time = std::chrono::system_clock::now() - start;
  return ret;
}

} // namespace query

namespace format {

void PrintHeaderTabular(const std::vector<std::string> &data, int total_width,
                        int column_width, int num_columns, bool all_columns_fit,
                        int margin = 1) {
  if (!all_columns_fit)
    num_columns -= 1;
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
uint64_t GetMaxColumnWidth(const mg_memory::MgListPtr &data, int margin = 1) {
  uint64_t column_width = 0;
  for (uint32_t i = 0; i < mg_list_size(data.get()); ++i) {
    std::stringstream field;
    utils::PrintValue(field, mg_list_at(data.get(), i));
    column_width = std::max(
        column_width, static_cast<uint64_t>(field.str().size() + 2 * margin));
  }
  return column_width + 1;
}

uint64_t GetMaxColumnWidth(const std::vector<std::string> &data,
                           int margin = 1) {
  uint64_t column_width = 0;
  for (const auto &field : data) {

    column_width = std::max(column_width,
                            static_cast<uint64_t>(field.size() + 2 * margin));
  }
  return column_width + 1;
}

void PrintRowTabular(const mg_memory::MgListPtr &data, int total_width,
                     int column_width, int num_columns, bool all_columns_fit,
                     int margin = 1) {
  if (!all_columns_fit)
    num_columns -= 1;
  std::string data_output = std::string(total_width, ' ');
  for (auto i = 0; i < total_width; i += column_width) {
    data_output[i] = '|';
    int idx = i / column_width;
    if (idx < num_columns) {
      std::stringstream field;
      utils::PrintValue(field,
                        mg_list_at(data.get(), idx)); // convert Value to string
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

void PrintTabular(const std::vector<std::string> &header,
                  const std::vector<mg_memory::MgListPtr> &records,
                  const bool fit_to_screen) {
  // lifted from replxx io.cxx
  auto get_screen_columns = []() {
    int cols(0);
#ifdef _WIN32
    HANDLE console_out = GetStdHandle(STD_OUTPUT_HANDLE);
    CONSOLE_SCREEN_BUFFER_INFO inf;
    GetConsoleScreenBufferInfo(console_out, &inf);
    cols = inf.dwSize.X;
#else
    struct winsize ws;
    cols = (ioctl(STDOUT_FILENO, TIOCGWINSZ, &ws) == -1) ? 80 : ws.ws_col;
#endif
    // cols is 0 in certain circumstances like inside debugger, which creates
    // further issues
    return (cols > 0) ? cols : 80;
  };

  auto window_columns = get_screen_columns();
  bool all_columns_fit = true;

  auto num_columns = header.size();
  auto column_width = GetMaxColumnWidth(header);
  for (size_t i = 0; i < records.size(); ++i) {
    column_width = std::max(column_width, GetMaxColumnWidth(records[i]));
  }
  column_width = std::max(static_cast<uint64_t>(5),
                          column_width); // set column width to min 5
  auto total_width = column_width * num_columns + 1;

  // Fit to screen width.
  if (fit_to_screen && total_width > window_columns) {
    uint64_t lo = 5;
    uint64_t hi = column_width;
    uint64_t last = 5;
    while (lo < hi) {
      uint64_t mid = lo + (hi - lo) / 2;
      uint64_t width = mid * num_columns + 1;
      if (width <= window_columns) {
        last = mid;
        lo = mid + 1;
      } else {
        hi = mid - 1;
      }
    }
    column_width = last;
    total_width = column_width * num_columns + 1;
    // All columns do not fit on screen.
    while (total_width > window_columns && num_columns > 1) {
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

std::vector<std::string> FormatCsvFields(const mg_memory::MgListPtr &fields,
                                         const CsvOptions &csv_opts) {
  std::vector<std::string> formatted;
  formatted.reserve(mg_list_size(fields.get()));
  for (uint32_t i = 0; i < mg_list_size(fields.get()); ++i) {
    std::stringstream field_stream;
    utils::PrintValue(field_stream, mg_list_at(fields.get(), i));
    std::string formatted_field(field_stream.str());
    if (csv_opts.doublequote) {
      formatted_field = utils::Replace(formatted_field, "\"", "\"\"");
    } else {
      formatted_field =
          utils::Replace(formatted_field, "\"", csv_opts.escapechar + "\"");
    }
    formatted_field.insert(0, 1, '"');
    formatted_field.append(1, '"');
    formatted.push_back(formatted_field);
  }
  return formatted;
}

std::vector<std::string> FormatCsvHeader(const std::vector<std::string> &fields,
                                         const CsvOptions &csv_opts) {
  std::vector<std::string> formatted;
  formatted.reserve(fields.size());
  for (auto formatted_field : fields) {
    if (csv_opts.doublequote) {
      formatted_field = utils::Replace(formatted_field, "\"", "\"\"");
    } else {
      formatted_field =
          utils::Replace(formatted_field, "\"", csv_opts.escapechar + "\"");
    }
    formatted_field.insert(0, 1, '"');
    formatted_field.append(1, '"');
    formatted.push_back(formatted_field);
  }
  return formatted;
}

void PrintCsv(const std::vector<std::string> &header,
              const std::vector<mg_memory::MgListPtr> &records,
              const CsvOptions &csv_opts) {
  // Print Header.
  auto formatted_header = FormatCsvHeader(header, csv_opts);
  utils::PrintIterable(std::cout, formatted_header, csv_opts.delimiter);
  std::cout << std::endl;
  // Print Records.
  for (size_t i = 0; i < records.size(); ++i) {
    auto formatted_row = FormatCsvFields(records[i], csv_opts);
    utils::PrintIterable(std::cout, formatted_row, csv_opts.delimiter);
    std::cout << std::endl;
  }
}

void Output(const std::vector<std::string> &header,
            const std::vector<mg_memory::MgListPtr> &records,
            const OutputOptions &out_opts, const CsvOptions &csv_opts) {
  if (out_opts.output_format == constants::kTabularFormat) {
    PrintTabular(header, records, out_opts.fit_to_screen);
  } else if (out_opts.output_format == constants::kCsvFormat) {
    PrintCsv(header, records, csv_opts);
  }
}

} // namespace format

namespace {
// Completion and syntax highlighting support

std::vector<std::string> GetCompletions(const char *text) {
  std::vector<std::string> matches;

  // Collect a vector of matches: vocabulary words that begin with text.
  std::string text_str = utils::ToUpperCase(std::string(text));
  for (auto word : constants::kCypherKeywords) {
    if (word.size() >= text_str.size() &&
        word.compare(0, text_str.size(), text_str) == 0) {
      matches.emplace_back(word);
    }
  }
  for (auto word : constants::kMemgraphKeywords) {
    if (word.size() >= text_str.size() &&
        word.compare(0, text_str.size(), text_str) == 0) {
      matches.emplace_back(word);
    }
  }
  for (auto word : constants::kAwesomeFunctions) {
    if (word.size() >= text_str.size() &&
        word.compare(0, text_str.size(), text_str) == 0) {
      matches.emplace_back(word);
    }
  }

  return matches;
}

void AddCompletions(replxx_completions *completions, const char *text) {
  auto text_completions = GetCompletions(text);
  for (const auto &completion : text_completions) {
    replxx_add_completion(completions, strdup(completion.c_str()));
  }
}

// taken from the replxx example.c (correct handling of UTF-8)
int utf8str_codepoint_length(char const *s, int utf8len) {
  int codepointLen = 0;
  unsigned char m4 = 128 + 64 + 32 + 16;
  unsigned char m3 = 128 + 64 + 32;
  unsigned char m2 = 128 + 64;
  for (int i = 0; i < utf8len; ++i, ++codepointLen) {
    char c = s[i];
    if ((c & m4) == m4) {
      i += 3;
    } else if ((c & m3) == m3) {
      i += 2;
    } else if ((c & m2) == m2) {
      i += 1;
    }
  }
  return codepointLen;
};

static char const wb[] = " \t\n\r\v\f-=+*&^%$#@!,./?<>;:`~'\"[]{}()\\|";

int context_length(char const *prefix) {
  // word boundary chars
  int i = (int)strlen(prefix) - 1;
  int cl = 0;
  while (i >= 0) {
    if (strchr(wb, prefix[i]) != NULL) {
      break;
    }
    ++cl;
    --i;
  }
  return cl;
};

void CompletionHook(const char *input, replxx_completions *completions,
                    int *contextLen, void *) {
  int utf8_context_len = context_length(input);
  int prefix_len = (int)strlen(input) - utf8_context_len;
  *contextLen = utf8str_codepoint_length(input + prefix_len, utf8_context_len);

  AddCompletions(completions, input + prefix_len);
}

ReplxxColor GetWordColor(const std::string_view word) {
  auto word_uppercase = utils::ToUpperCase(std::string(word));
  bool is_cypher_keyword =
      std::find(constants::kCypherKeywords.begin(),
                constants::kCypherKeywords.end(),
                word_uppercase) != constants::kCypherKeywords.end();
  bool is_memgraph_keyword =
      std::find(constants::kMemgraphKeywords.begin(),
                constants::kMemgraphKeywords.end(),
                word_uppercase) != constants::kMemgraphKeywords.end();
  bool is_awesome_function =
      std::find(constants::kAwesomeFunctions.begin(),
                constants::kAwesomeFunctions.end(),
                word_uppercase) != constants::kAwesomeFunctions.end();
  if (is_cypher_keyword || is_memgraph_keyword) {
    return REPLXX_COLOR_YELLOW;
  } else if (is_awesome_function) {
    return REPLXX_COLOR_BRIGHTRED;
  } else {
    return REPLXX_COLOR_DEFAULT;
  }
}

void SetWordColor(std::string_view word, ReplxxColor *colors,
                  int *colors_offset) {
  auto color = GetWordColor(word);
  auto word_codepoint_len = utf8str_codepoint_length(word.data(), word.size());
  for (int i = *colors_offset; i < *colors_offset + word_codepoint_len; i++) {
    colors[i] = color;
  }
  // +1 for the word boundary char (we don't want to color it)
  *colors_offset = *colors_offset + word_codepoint_len + 1;
}

void ColorHook(const char *input, ReplxxColor *colors, int size, void *) {
  auto *word_begin = input;
  size_t word_size = 0;
  int colors_offset = 0;

  auto input_size = strlen(input);
  for (int i = 0; i < input_size; i++) {
    // word boundary
    if (strchr(wb, input[i]) != NULL) {
      SetWordColor(std::string_view(word_begin, word_size), colors,
                   &colors_offset);
      // if the boundary is not the last char in input, advance the
      // next word ptr and reset word size
      if (i != (input_size - 1)) {
        word_begin = input + i + 1;
        word_size = 0;
      }
      // end of input
    } else if (i == (input_size - 1)) {
      // regular char encountered as the last char of input
      word_size++;
      SetWordColor(std::string_view(word_begin, word_size), colors,
                   &colors_offset);
    } else {
      // regular char encountered, but not the end of input - advance
      word_size++;
    }
  }
}

} // namespace

DECLARE_bool(term_colors);

Replxx *InitAndSetupReplxx() {
  Replxx *replxx_instance = replxx_init();

  replxx_set_unique_history(replxx_instance, 1);
  replxx_set_completion_callback(replxx_instance, CompletionHook, nullptr);

  // ToDo(the-joksim):
  //   - syntax highlighting disabled for now - figure out a smarter way of
  //     picking the right colors depending on the user's terminal settings
  //   - currently, the color scheme for highlighting is hardcoded
  if (FLAGS_term_colors) {
    replxx_set_highlighter_callback(replxx_instance, ColorHook, nullptr);
  }

  return replxx_instance;
}
