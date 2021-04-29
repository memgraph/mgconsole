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
#include <iostream>
#include <optional>
#include <thread>

#include <pwd.h>
#include <signal.h>
#include <termios.h>
#include <unistd.h>

#include <gflags/gflags.h>

#include "replxx.h"

#include "mgclient.h"
#include "version.hpp"
#include "utils/utils.hpp"
#include "utils/constants.hpp"

using namespace std::string_literals;

volatile sig_atomic_t is_shutting_down = 0;

// connection
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
  if (value == constants::kCsvFormat || value == constants::kTabularFormat) {
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
DEFINE_string(
    csv_escapechar, "",
    "Character used to escape the quotechar(\") if csv-doublequote is false.");
DEFINE_bool(
    csv_doublequote, true,
    "Controls how instances of quotechar(\") appearing inside a field should "
    "themselves be quoted. When true, the character is doubled. When false, "
    "the escapechar is used as a prefix to the quotechar. "
    "If csv-doublequote is false 'csv-escapechar' must be set.");

// history
DEFINE_string(history, "~/.memgraph",
              "Use the specified directory for saving history.");
DEFINE_bool(no_history, false, "Do not save history.");

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

  Replxx *replxx_instance = InitAndSetupReplxx();

  auto password = FLAGS_password;
  if (isatty(STDIN_FILENO) && FLAGS_username.size() > 0 &&
      password.size() == 0) {
    console::SetStdinEcho(false);
    auto password_optional = console::ReadLine(replxx_instance, "Password: ");
    std::cout << std::endl;
    if (password_optional) {
      password = *password_optional;
    } else {
      console::EchoFailure("Password not submitted",
                  "Requested password for username " + FLAGS_username);
      return 1;
    }
    console::SetStdinEcho(true);
  }


  fs::path history_dir = FLAGS_history;
  if (FLAGS_history ==
      (constants::kDefaultHistoryBaseDir + "/" + constants::kDefaultHistoryMemgraphDir)) {
    // Fetch home dir for user.
    struct passwd *pw = getpwuid(getuid());
    history_dir = fs::path(pw->pw_dir) / constants::kDefaultHistoryMemgraphDir;
  }
  if (!utils::EnsureDir(history_dir)) {
    console::EchoFailure("History directory doesn't exist", history_dir);
    // Should program exit here or just continue with warning message?
    return 1;
  }
  fs::path history_file = history_dir / constants::kHistoryFilename;
  // Read history file.
  if (fs::exists(history_file)) {
    auto ret =
        replxx_history_load(replxx_instance, history_file.string().c_str());
    if (ret != 0) {
      console::EchoFailure("Unable to read history file", history_file);
      // Should program exit here or just continue with warning message?
      return 1;
    }
  }

  // Save history function. Used to save readline history after each query.
  auto save_history = [history_file, replxx_instance] {
    if (!FLAGS_no_history) {
      // If there was no history, create history file.
      // Otherwise, append to existing history.
      auto ret =
          replxx_history_save(replxx_instance, history_file.string().c_str());
      if (ret != 0) {
        console::EchoFailure("Unable to save history to file", history_file);
        return 1;
      }
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

  mg_memory::MgSessionParamsPtr params =
      mg_memory::MakeCustomUnique<mg_session_params>(mg_session_params_make());
  if (!params) {
    console::EchoFailure("Connection failure",
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

  mg_memory::MgSessionPtr session = mg_memory::MakeCustomUnique<mg_session>(nullptr);
  {
    mg_session *session_tmp;
    int status = mg_connect(params.get(), &session_tmp);
    session = mg_memory::MakeCustomUnique<mg_session>(session_tmp);
    if (status != 0) {
      console::EchoFailure("Connection failure", mg_session_error(session.get()));
      return 1;
    }
  }

  console::EchoInfo("mgconsole "s + gflags::VersionString());
  console::EchoInfo("Type :help for shell usage");
  console::EchoInfo("Quit the shell by typing Ctrl-D(eof) or :quit");
  console::EchoInfo("Connected to 'memgraph://" + FLAGS_host + ":" +
           std::to_string(FLAGS_port) + "'");
  int num_retries = 3;
  while (true) {
    auto query = query::GetQuery(replxx_instance);
    if (!query) {
      console::EchoInfo("Bye");
      break;
    }
    if (query->empty()) continue;
    try {
      auto ret = query::ExecuteQuery(session.get(), *query);
      if (ret.records.size() > 0) Output(ret.header, ret.records, output_opts, csv_opts);
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
        console::EchoFailure("Failed query", *query);
      }
      console::EchoFailure("Client received exception", e.what());
      if (!isatty(STDIN_FILENO)) {
        return 1;
      }
    } catch (const utils::ClientFatalException &e) {
      console::EchoFailure("Client received exception", e.what());
      console::EchoInfo("Trying to reconnect");
      bool is_connected = false;
      session.reset(nullptr);
      while (num_retries > 0) {
        --num_retries;
        mg_session *session_tmp;
        int status = mg_connect(params.get(), &session_tmp);
        session = mg_memory::MakeCustomUnique<mg_session>(session_tmp);
        if (status != 0) {
          console::EchoFailure("Connection failure", mg_session_error(session.get()));
          session.reset(nullptr);
        } else {
          is_connected = true;
          break;
        }
        std::this_thread::sleep_for(std::chrono::seconds(1));
      }
      if (is_connected) {
        num_retries = 3;
        console::EchoInfo("Connected to 'memgraph://" + FLAGS_host + ":" +
                 std::to_string(FLAGS_port) + "'");
      } else {
        console::EchoFailure("Couldn't connect to", "'memgraph://" + FLAGS_host + ":" +
                                               std::to_string(FLAGS_port) +
                                               "'");
        return 1;
      }
    }
  }
  return 0;
}
