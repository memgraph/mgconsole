// mgconsole - console client for Memgraph database
//
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

#include <signal.h>

#include <algorithm>
#include <cstdio>
#include <iostream>
#include <optional>
#include <thread>
#include <unordered_map>

#ifdef _WIN32

#include <cstdlib>

#else /* _WIN32 */

#include <pwd.h>
#include <termios.h>
#include <unistd.h>

#endif /* _WIN32 */

#include <gflags/gflags.h>
#include <mgclient.h>
#include <replxx.h>

#include "batch_import.hpp"
#include "interactive.hpp"
#include "parsing.hpp"
#include "serial_import.hpp"
#include "utils/assert.hpp"
#include "utils/constants.hpp"
#include "utils/utils.hpp"
#include "version.hpp"

using namespace std::string_literals;

volatile sig_atomic_t is_shutting_down = 0;

// connection
DEFINE_string(host, "127.0.0.1", "Server address. It can be a DNS resolvable hostname.");
DEFINE_int32(port, 7687, "Server port.");
DEFINE_string(username, "", "Database username.");
DEFINE_string(password, "", "Database password.");
DEFINE_bool(use_ssl, false, "Use SSL when connecting to the server.");

// output
DEFINE_bool(fit_to_screen, false, "Fit output width to screen width.");
DEFINE_bool(term_colors, false, "Use terminal colors syntax highlighting.");
DEFINE_string(output_format, "tabular",
              "Query output format can be csv, tabular or cypherl. If output format is "
              "not tabular `fit-to-screen` flag is ignored.");
DEFINE_bool(verbose_execution_info, false,
            "Output the additional information about query such as query cost, parsing, planning and execution times.");
DEFINE_validator(output_format, [](const char *, const std::string &value) {
  if (value == constants::kCsvFormat || value == constants::kTabularFormat || value == constants::kCypherlFormat) {
    return true;
  }
  return false;
});

// CSV
DEFINE_string(csv_delimiter, ",", "Character used to separate fields.");
DEFINE_validator(csv_delimiter, [](const char *, const std::string &value) {
  if (value.size() != 1) {
    return false;
  }
  return true;
});
DEFINE_string(csv_escapechar, "", "Character used to escape the quotechar(\") if the `csv-doublequote` flag is false.");
DEFINE_bool(csv_doublequote, true,
            "Controls how instances of quotechar(\") appearing inside a field should "
            "themselves be quoted. When true, the character is doubled. When false, "
            "the escapechar is used as a prefix to the quotechar. "
            "If `csv-doublequote` is false, 'csv-escapechar' must be set.");

// history
DEFINE_string(history, "~/.memgraph", "Use the specified directory to save history.");
DEFINE_bool(no_history, false, "Do not save history.");

DEFINE_string(
    import_mode, "serial",
    "Import mode defines the way how the queries will be executed. `serial` mode will try to execute queries in the "
    "specified input order. `batched-parallel` will batched and parallelize query execution. NOTE: batched-parallel is "
    "an experimental feature, the behavior might be unexpected because it depends on how the underlying database "
    "system is configured (e.g., in the transactional setup there might be many serialization errors, while in the "
    "analytical setup, ordering of nodes/edges is very important. `parser` mode will just print info about the "
    "provided queries. NOTE: `parser` mode won't execute any query against the underlying database system.");
DEFINE_validator(import_mode, [](const char *, const std::string &value) {
  if (value == constants::kSerialMode || value == constants::kBatchedParallel || value == constants::kParserMode) {
    return true;
  }
  return false;
});
DEFINE_int32(batch_size, 1000, "A single batch size only when --import-mode=batched-parallel.");
DEFINE_int32(workers_number, 32,
             "The number of threads to execute batches in parallel, only when --import-mode=batched-parallel");
DEFINE_bool(collect_parser_stats, true, "Collect parsing statistics only when --import-mode=parser");
DEFINE_bool(print_parser_stats, true, "Print parser statistics for each query only when --import-mode=parser");

DECLARE_int32(min_log_level);

int main(int argc, char **argv) {
  gflags::SetVersionString(version_string);
  gflags::SetUsageMessage(constants::kUsage);

  gflags::ParseCommandLineFlags(&argc, &argv, true);

  format::CsvOptions csv_opts{FLAGS_csv_delimiter, FLAGS_csv_escapechar, FLAGS_csv_doublequote};
  format::OutputOptions output_opts{FLAGS_output_format, FLAGS_fit_to_screen};

  if (output_opts.output_format == constants::kCsvFormat && !csv_opts.ValidateDoubleQuote()) {
    console::EchoFailure(
        "Unsupported combination of 'csv-doublequote' and 'csv-escapechar'\n"
        "flags",
        "Run '" + std::string(argv[0]) + " --help' for usage.");
    return 1;
  }

  if (mg_init() != 0) {
    console::EchoFailure("Internal error", "Couldn't initialize all the resources");
    return 1;
  }

#ifdef _WIN32
// ToDo(the-joksim):
//  - How to handle shutdown inside a shutdown on Windows? (Windows uses
//  messages instead of signals.)
//  - the double shutdown (a second signal being sent to the process while the
//  first
//    signal is being handled, both signals causing process termination) should
//    be a rare event (SIGTERM might be sent from other processes, such as
//    daemons/services, e.g. when the system is shutting down). What behavior
//    does the double shutdown cause, and what's the benefit in handling it?
#else /* _WIN32 */

  auto shutdown = [](int exit_code = 0) {
    if (is_shutting_down) return;
    is_shutting_down = 1;

#ifdef __APPLE__

    std::exit(exit_code);

#else /* __APPLE__ */

    std::quick_exit(exit_code);

#endif /*__APPLE__*/
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

#endif /* _WIN32 */

  utils::bolt::Config bolt_config{
      .host = FLAGS_host,
      .port = FLAGS_port,
      .username = FLAGS_username,
      .password = FLAGS_password,
      .use_ssl = FLAGS_use_ssl,
  };

  if (console::is_a_tty(STDIN_FILENO)) {  // INTERACTIVE
    return mode::interactive::Run(bolt_config, FLAGS_history, FLAGS_no_history, FLAGS_verbose_execution_info, csv_opts,
                                  output_opts);
  } else if (FLAGS_import_mode == constants::kParserMode) {
    return mode::parsing::Run(FLAGS_collect_parser_stats, FLAGS_print_parser_stats);
  } else if (FLAGS_import_mode == constants::kBatchedParallel) {
    return mode::batch_import::Run(bolt_config, FLAGS_batch_size, FLAGS_workers_number);
  } else if (FLAGS_import_mode == constants::kSerialMode) {
    return mode::serial_import::Run(bolt_config, csv_opts, output_opts);
  } else {
    MG_FAIL("Unknown import mode!");
  }

  return 0;
}
