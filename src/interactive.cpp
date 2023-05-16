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

#include "interactive.hpp"

#include <thread>

#include <gflags/gflags.h>

#include "utils/constants.hpp"

namespace mode::interactive {

using namespace std::string_literals;

int Run(const utils::bolt::Config &bolt_config, const std::string &history, bool no_history,
        bool verbose_execution_info, const format::CsvOptions &csv_opts, const format::OutputOptions &output_opts) {
  Replxx *replxx_instance = InitAndSetupReplxx();

  bool resources_cleaned_up = false;
  auto cleanup_resources = [replxx_instance, &resources_cleaned_up]() {
    if (!resources_cleaned_up) {
      replxx_end(replxx_instance);
      resources_cleaned_up = true;
    }
  };

  auto password = bolt_config.password;
  if (bolt_config.username.size() > 0 && password.size() == 0) {
    console::SetStdinEcho(false);
    auto password_optional = console::ReadLine(replxx_instance, "Password: ");
    std::cout << std::endl;
    if (password_optional) {
      password = *password_optional;
    } else {
      console::EchoFailure("Password not submitted", "Requested password for username " + bolt_config.username);
      cleanup_resources();
      return 1;
    }
    console::SetStdinEcho(true);
  }

  fs::path history_dir = history;
  if (history == (constants::kDefaultHistoryBaseDir + "/" + constants::kDefaultHistoryMemgraphDir)) {
    // Fetch home dir for user.
    history_dir = utils::GetUserHomeDir() / constants::kDefaultHistoryMemgraphDir;
  }
  if (!utils::EnsureDir(history_dir)) {
    console::EchoFailure("History directory doesn't exist", history_dir.string());
    // Should program exit here or just continue with warning message?
    cleanup_resources();
    return 1;
  }
  fs::path history_file = history_dir / constants::kHistoryFilename;
  // Read history file.
  if (fs::exists(history_file)) {
    auto ret = replxx_history_load(replxx_instance, history_file.string().c_str());
    if (ret != 0) {
      console::EchoFailure("Unable to read history file", history_file.string());
      // Should program exit here or just continue with warning message?
      cleanup_resources();
      return 1;
    }
  }

  // Save history function. Used to save replxx history after each query.
  auto save_history = [&history_file, replxx_instance, &cleanup_resources, no_history] {
    if (!no_history) {
      // If there was no history, create history file.
      // Otherwise, append to existing history.
      auto ret = replxx_history_save(replxx_instance, history_file.string().c_str());
      if (ret != 0) {
        console::EchoFailure("Unable to save history to file", history_file.string());
        cleanup_resources();
        return 1;
      }
    }
    return 0;
  };

  int num_retries = 3;
  auto session = MakeBoltSession(bolt_config);
  if (session.get() == nullptr) {
    cleanup_resources();
    return 1;
  }

  console::EchoInfo("mgconsole "s + gflags::VersionString());
  console::EchoInfo("Connected to 'memgraph://" + bolt_config.host + ":" + std::to_string(bolt_config.port) + "'");
  console::EchoInfo("Type :help for shell usage");
  console::EchoInfo("Quit the shell by typing Ctrl-D(eof) or :quit");

  while (true) {
    auto query = query::GetQuery(replxx_instance);
    if (!query) {
      console::EchoInfo("Bye");
      break;
    }
    if (query->query.empty()) {
      continue;
    }

    try {
      auto ret = query::ExecuteQuery(session.get(), query->query);
      if (ret.records.size() > 0) {
        Output(ret.header, ret.records, output_opts, csv_opts);
      }
      std::string summary;
      if (ret.records.size() == 0) {
        summary = "Empty set";
      } else if (ret.records.size() == 1) {
        summary = std::to_string(ret.records.size()) + " row in set";
      } else {
        summary = std::to_string(ret.records.size()) + " rows in set";
      }
      std::printf("%s (round trip in %.3lf sec)\n", summary.c_str(), ret.wall_time.count());
      auto history_ret = save_history();
      if (history_ret != 0) {
        cleanup_resources();
        return history_ret;
      }
      if (ret.notification) {
        console::EchoNotification(ret.notification.value());
      }
      if (ret.stats) {
        console::EchoStats(ret.stats.value());
      }
      if (verbose_execution_info && ret.execution_info) {
        console::EchoExecutionInfo(ret.execution_info.value());
      }
    } catch (const utils::ClientQueryException &e) {
      console::EchoFailure("Client received query exception", e.what());
    } catch (const utils::ClientFatalException &e) {
      console::EchoFailure("Client received connection exception", e.what());
      console::EchoInfo("Trying to reconnect...");
      bool is_connected = false;
      session.reset(nullptr);
      while (num_retries > 0) {
        --num_retries;
        session = utils::bolt::MakeBoltSession(bolt_config);
        if (session.get() == nullptr) {
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
        console::EchoInfo("Connected to 'memgraph://" + bolt_config.host + ":" + std::to_string(bolt_config.port) +
                          "'");
      } else {
        console::EchoFailure("Couldn't connect to",
                             "'memgraph://" + bolt_config.host + ":" + std::to_string(bolt_config.port) + "'");
        cleanup_resources();
        return 1;
      }
    }
  }

  cleanup_resources();
  return 0;
}

}  // namespace mode::interactive
