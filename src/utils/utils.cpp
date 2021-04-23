#include <algorithm>
#include <iostream>

#include <sys/ioctl.h>
#include <unistd.h>
#include <termios.h>

#include <readline/history.h>
#include <readline/readline.h>

#include "constants.hpp"
#include "utils.hpp"

namespace utils {

bool EnsureDir(const fs::path &dir) noexcept {
  std::error_code error_code;  // for exception suppression.
  if (fs::exists(dir, error_code)) return fs::is_directory(dir, error_code);
  return fs::create_directories(dir, error_code);
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

std::string Replace(std::string src, const std::string &match, const std::string &replacement) {
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
    default:
      os << "{unknown value}";
      break;
  }
}

}  // namespace utils

namespace console {

void PrintHelp() { std::cout << constants::kInteractiveUsage << std::endl; }

void EchoFailure(const std::string &failure_msg, const std::string &explanation) {
  if (isatty(STDIN_FILENO)) {
    std::cout << "\033[1;31m" << failure_msg << ": \033[0m";
    std::cout << explanation << std::endl;
  } else {
    std::cerr << failure_msg << ": ";
    std::cerr << explanation << std::endl;
  }
}

void EchoInfo(const std::string &message) {
  if (isatty(STDIN_FILENO)) {
    std::cout << message << std::endl;
  }
}

void SetStdinEcho(bool enable = true) {
  struct termios tty;
  tcgetattr(STDIN_FILENO, &tty);
  if (!enable) {
    tty.c_lflag &= ~ECHO;
  } else {
    tty.c_lflag |= ECHO;
  }
  tcsetattr(STDIN_FILENO, TCSANOW, &tty);
}

int SetDefaultText() {
  rl_insert_text(default_text.c_str());
  default_text = "";
  rl_startup_hook = (rl_hook_func_t *)NULL;
  return 0;
}


std::experimental::optional<std::string> GetLine() {
  std::string line;
  std::getline(std::cin, line);
  if (std::cin.eof()) return std::experimental::nullopt;
  line = default_text + line;
  default_text = "";
  return line;
}


std::pair<std::string, bool> ParseLine(const std::string &line, char *quote, bool *escaped) {
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

std::experimental::optional<std::string> ReadLine(const std::string &prompt) {
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

}  // namespace console


namespace query {

std::experimental::optional<std::string> GetQuery() {
  char quote = '\0';
  bool escaped = false;
  auto ret = console::ParseLine(default_text, &quote, &escaped);
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
      line = console::GetLine();
    } else {
      line = console::ReadLine(line_cnt == 0 ? constants::kPrompt : constants::kMultilinePrompt);
      if (line_cnt == 0 && line && line->size() > 0 && (*line)[0] == ':') {
        auto trimmed_line = utils::Trim(*line);
        if (trimmed_line == constants::kCommandQuit) {
          return std::experimental::nullopt;
        } else if (trimmed_line == constants::kCommandHelp) {
          console::PrintHelp();
          return "";
        } else {
          console::EchoFailure("Unsupported command", trimmed_line);
          console::PrintHelp();
          return "";
        }
      }
    }
    if (!line) return std::experimental::nullopt;
    if (line->empty()) continue;
    auto ret = console::ParseLine(*line, &quote, &escaped);
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
        mg_memory::MakeCustomUnique<mg_list>(mg_list_copy(mg_result_row(result))));
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

  ret.wall_time = std::chrono::system_clock::now() - start;
  return ret;
}

}   //namespace query

namespace format {

void PrintHeaderTabular(const std::vector<std::string> &data, int total_width, int column_width,
                        int num_columns, bool all_columns_fit, int margin = 1) {
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
uint64_t GetMaxColumnWidth(const mg_memory::MgListPtr &data, int margin = 1) {
  uint64_t column_width = 0;
  for (uint32_t i = 0; i < mg_list_size(data.get()); ++i) {
    std::stringstream field;
    utils::PrintValue(field, mg_list_at(data.get(), i));
    column_width = std::max(column_width, field.str().size() + 2 * margin);
  }
  return column_width + 1;
}

uint64_t GetMaxColumnWidth(const std::vector<std::string> &data, int margin = 1) {
  uint64_t column_width = 0;
  for (const auto &field : data) {
    column_width = std::max(column_width, field.size() + 2 * margin);
  }
  return column_width + 1;
}

void PrintRowTabular(const mg_memory::MgListPtr &data, int total_width,
                     int column_width, int num_columns,
                     bool all_columns_fit, int margin = 1) {
  if (!all_columns_fit) num_columns -= 1;
  std::string data_output = std::string(total_width, ' ');
  for (auto i = 0; i < total_width; i += column_width) {
    data_output[i] = '|';
    int idx = i / column_width;
    if (idx < num_columns) {
      std::stringstream field;
      utils::PrintValue(field,
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

void PrintTabular(const std::vector<std::string> &header,
                  const std::vector<mg_memory::MgListPtr> &records, const bool fit_to_screen) {
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
  if (fit_to_screen && total_width > w.ws_col) {
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

std::vector<std::string> FormatCsvFields(const mg_memory::MgListPtr &fields, const CsvOptions &csv_opts) {
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

std::vector<std::string> FormatCsvHeader(const std::vector<std::string> &fields, const CsvOptions &csv_opts) {
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
            const OutputOptions &out_opts,
            const CsvOptions &csv_opts) {
  if (out_opts.output_format == constants::kTabularFormat) {
    PrintTabular(header, records, out_opts.fit_to_screen);
  } else if (out_opts.output_format == constants::kCsvFormat) {
    PrintCsv(header, records, csv_opts);
  }
}

}  // namespace format
