// mgconsole - console client for Memgraph database
// Copyright (C) 2016-2021 Memgraph Ltd. [https://memgraph.com]
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
#include <cstdio>
#include <iostream>
#include <optional>
#include <thread>
#include <random>
#include <unordered_map>

#include <signal.h>

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

#include "utils/constants.hpp"
#include "utils/utils.hpp"
#include "utils/thread_pool.hpp"
#include "utils/notifier.hpp"
#include "utils/future.hpp"
#include "version.hpp"

using namespace std::string_literals;

volatile sig_atomic_t is_shutting_down = 0;

// connection
DEFINE_string(host, "127.0.0.1", "Server address. It can be a DNS resolvable hostname.");
DEFINE_int32(port, 7687, "Server port");
DEFINE_string(username, "", "Username for the database");
DEFINE_string(password, "", "Password for the database");
DEFINE_bool(use_ssl, false, "Use SSL when connecting to the server.");

DEFINE_bool(fit_to_screen, false, "Fit output width to screen width.");
DEFINE_bool(term_colors, false, "Use terminal colors syntax highlighting.");
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
DEFINE_string(csv_escapechar, "", "Character used to escape the quotechar(\") if csv-doublequote is false.");
DEFINE_bool(csv_doublequote, true,
            "Controls how instances of quotechar(\") appearing inside a field should "
            "themselves be quoted. When true, the character is doubled. When false, "
            "the escapechar is used as a prefix to the quotechar. "
            "If csv-doublequote is false 'csv-escapechar' must be set.");

// history
DEFINE_string(history, "~/.memgraph", "Use the specified directory for saving history.");
DEFINE_bool(no_history, false, "Do not save history.");

DECLARE_int32(min_log_level);

mg_memory::MgSessionPtr MakeBoltSession(const std::string& password) {
  std::string bolt_client_version = "mg/"s + gflags::VersionString();
  mg_memory::MgSessionParamsPtr params = mg_memory::MakeCustomUnique<mg_session_params>(mg_session_params_make());
  if (!params) {
    console::EchoFailure("Connection failure", "out of memory, failed to allocate `mg_session_params` struct");
  }
  mg_session_params_set_host(params.get(), FLAGS_host.c_str());
  mg_session_params_set_port(params.get(), FLAGS_port);
  if (!FLAGS_username.empty()) {
    mg_session_params_set_username(params.get(), FLAGS_username.c_str());
    mg_session_params_set_password(params.get(), password.c_str());
  }
  mg_session_params_set_user_agent(params.get(), bolt_client_version.c_str());
  mg_session_params_set_sslmode(params.get(), FLAGS_use_ssl ? MG_SSLMODE_REQUIRE : MG_SSLMODE_DISABLE);
  mg_memory::MgSessionPtr session = mg_memory::MakeCustomUnique<mg_session>(nullptr);
  {
    mg_session *session_tmp;
    int status = mg_connect(params.get(), &session_tmp);
    session = mg_memory::MakeCustomUnique<mg_session>(session_tmp);
    if (status != 0) {
      // TODO(gitbuda): The console echo should be moved outside MakeBoltSession
      console::EchoFailure("Connection failure", mg_session_error(session.get()));
      return mg_memory::MakeCustomUnique<mg_session>(nullptr);
    }
    return session;
  }
  return session;
}

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
  Replxx *replxx_instance = InitAndSetupReplxx();

  bool resources_cleaned_up = false;
  auto cleanup_resources = [replxx_instance, &resources_cleaned_up]() {
    if (!resources_cleaned_up) {
      replxx_end(replxx_instance);
      resources_cleaned_up = true;
    }
  };

  auto password = FLAGS_password;
  if (console::is_a_tty(STDIN_FILENO) && FLAGS_username.size() > 0 && password.size() == 0) {
    console::SetStdinEcho(false);
    auto password_optional = console::ReadLine(replxx_instance, "Password: ");
    std::cout << std::endl;
    if (password_optional) {
      password = *password_optional;
    } else {
      console::EchoFailure("Password not submitted", "Requested password for username " + FLAGS_username);
      cleanup_resources();
      return 1;
    }
    console::SetStdinEcho(true);
  }

  fs::path history_dir = FLAGS_history;
  if (FLAGS_history == (constants::kDefaultHistoryBaseDir + "/" + constants::kDefaultHistoryMemgraphDir)) {
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
  auto save_history = [&history_file, replxx_instance, &cleanup_resources] {
    if (!FLAGS_no_history) {
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

  auto session = MakeBoltSession(password);
  // TODO(gitbuda): Refactor to become cleaner.
  if (session.get() == nullptr) {
    cleanup_resources();
    return 1;
  }

  console::EchoInfo("mgconsole "s + gflags::VersionString());
  console::EchoInfo("Connected to 'memgraph://" + FLAGS_host + ":" + std::to_string(FLAGS_port) + "'");
  console::EchoInfo("Type :help for shell usage");
  console::EchoInfo("Quit the shell by typing Ctrl-D(eof) or :quit");

  // int num_retries = 3; // this is related to the network retries not serialization retries
  uint64_t batch_size = 1000;
  int64_t max_concurrent_executions = 16;
  uint64_t thread_pool_size = 16;
  // All queries have to be stored or at least N * batch_size
  int64_t batch_index = 0;
  query::Batch batch(batch_size, batch_index);
  std::vector<query::Batch> batches;
  utils::ThreadPool thread_pool(thread_pool_size);
  utils::Notifier notifier;

  while (true) {
    auto query = query::GetQuery(replxx_instance);
    if (!query) {
      console::EchoInfo("Bye");
      break;
    }
    if (query->empty()) {
      continue;
    }

    // TODO(gitbuda): Batch LIMITED number of queries (based on the thread pool) and execute all in parallel with retries.
    // TODO(gitbuda): Indexes are a problem because they can't be created in a multi-query transaction.
    if (!console::is_a_tty(STDIN_FILENO)) { // reading from STDIN -> BATCHING
      if (batch.queries.size() < batch_size) {
        batch.queries.emplace_back(query::Query{.line_number=0,.query_number=0,.query=*query});
      } else {
        batch_index += 1;
        batches.emplace_back(std::move(batch));
        batch = query::Batch(batch_size, batch_index);
        batch.queries.emplace_back(query::Query{.line_number=0,.query_number=0,.query=*query});
      }

    } else { // interactive mode -> NO BATCHING
      try {
        auto ret = query::ExecuteQuery(session.get(), *query);
        if (ret.records.size() > 0) {
          Output(ret.header, ret.records, output_opts, csv_opts);
        }
        if (console::is_a_tty(STDIN_FILENO)) {
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
        }
      } catch (const utils::ClientQueryException &e) {
        if (!console::is_a_tty(STDIN_FILENO)) {
          console::EchoFailure("Failed query", *query);
        }
        console::EchoFailure("Client received exception", e.what());
        if (!console::is_a_tty(STDIN_FILENO)) {
          cleanup_resources();
          return 1;
        }
      } catch (const utils::ClientFatalException &e) {
        console::EchoFailure("Client received exception", e.what());
        console::EchoInfo("Trying to reconnect");
        bool is_connected = false;
        session.reset(nullptr);

        // // TODO(gitbuda): Retry depends on the connection params! Refacto!
        // while (num_retries > 0) {
        //   --num_retries;
        //   mg_session *session_tmp;
        //   int status = mg_connect(params.get(), &session_tmp);
        //   session = mg_memory::MakeCustomUnique<mg_session>(session_tmp);
        //   if (status != 0) {
        //     console::EchoFailure("Connection failure", mg_session_error(session.get()));
        //     session.reset(nullptr);
        //   } else {
        //     is_connected = true;
        //     break;
        //   }
        //   std::this_thread::sleep_for(std::chrono::seconds(1));
        // }
        if (is_connected) {
          // num_retries = 3;
          console::EchoInfo("Connected to 'memgraph://" + FLAGS_host + ":" + std::to_string(FLAGS_port) + "'");
        } else {
          console::EchoFailure("Couldn't connect to",
                               "'memgraph://" + FLAGS_host + ":" + std::to_string(FLAGS_port) + "'");
          cleanup_resources();
          return 1;
        }
      }
    }
  }

  if (!batches.empty()) {
    // Add last batch if it's missing!
    if (batch.queries.size() > 0 && batch.queries.size() < batch_size) {
      batches.emplace_back(std::move(batch));
    }
    query::PrintBatchesInfo(batches);

    std::vector<std::thread> threads;
    threads.reserve(thread_pool_size);
    std::vector<mg_memory::MgSessionPtr> sessions;
    sessions.reserve(thread_pool_size);
    for (uint64_t thread_i = 0; thread_i < thread_pool_size; ++thread_i) {
      sessions[thread_i] = MakeBoltSession(password);
      // TODO(gitbuda): Handle failed connections required for the thread pool.
      if (!sessions[thread_i].get()) {
        std::cout << "a session uninitialized" << std::endl;
      }
    }
    std::cout << "all batching stuff initialized" << std::endl;

    std::random_device rd;
    std::default_random_engine rng(rd());
    std::uniform_int_distribution<> dist(2, 10);
    // dist(rng) // to generate random int
    // std::shuffle(batches.begin(), batches.end(), rng);
    query::PrintBatchesInfo(batches);

    // NOTE: memgraph's thread pool would fit here in an amazing way, but life
    //       thread_pool -> spin_lock -> as is now compiles only on Linux
    //
    // NOTE: A big problem with batched execution is that executing batches with
    //       edges will pass on an empty database (with no nodes) without an error:
    //         * edges have to come after nodes
    //         * count how many elements is actually created from a given batch
    //
    // NOTE: Tactics to deal with the retry of serialization errors
    //   * backoff
    //   * serial execution
    //
    // TODO(gitbuda): If this is executed multiple times -> bad session + unknown message type -> debug
    // TODO(gitbuda): The biggest issue seems to be that a few conflicting batches can end up in constant serialization conflict.
    // TODO(gitbuda): Figure out the right batched and parallel execution.

    std::atomic<uint64_t> executed_batches = 0;
    while (true) {
      std::cout << "EXECUTED BATCHES: " << executed_batches << std::endl;
      if (executed_batches.load() >= batches.size()) {
        break;
      }

      // // SERIAL EXECUTION to reduce the number of conflicting batches
      // for (uint64_t batch_i = 0; batch_i < batches.size(); ++batch_i) {
      //   auto &batch = batches.at(batch_i);
      //   std::cout << "Batch " << batch_i << " " << batch.attempts << std::endl;
      //   if (batch.is_executed) {
      //     continue;
      //   }
      //   std::cout << "Batch " << batch_i << " " << batch.attempts << std::endl;
      //   if (batch.attempts > 2) {
      //     auto ret = query::ExecuteBatch(session.get(), batch);
      //     if (ret.is_executed) {
      //       batch.is_executed = true;
      //       executed_batches++;
      //       std::cout << "batch: " << batch_i << " done (serial)" << std::endl;
      //     } else {
      //       std::cout << "batch FAILED " << batch_i << std::endl;
      //     }
      //   }
      //   // TODO(gitbuda): session can end up in a bad state -> debug and fix
      //   if (mg_session_status(session.get()) == MG_SESSION_BAD) {
      //     session = MakeBoltSession(password);
      //   }
      // }

      std::unordered_map<size_t, utils::Future<bool>> f_execs;
      int64_t used_threads = 0;
      for (uint64_t batch_i = 0; batch_i < batches.size(); ++batch_i) {
        if (used_threads >= max_concurrent_executions) {
          break;
        }
        auto &batch = batches.at(batch_i);
        if (batch.is_executed) {
          continue;
        }
        auto thread_i = used_threads;
        used_threads++;
        utils::ReadinessToken readiness_token{static_cast<size_t>(batch_i)};
        std::function<void()> fill_notifier = [readiness_token, &notifier]() { notifier.Notify(readiness_token); };
        auto [future, promise] = utils::FuturePromisePairWithNotifications<bool>(nullptr, fill_notifier);
        auto shared_promise = std::make_shared<decltype(promise)>(std::move(promise));
        thread_pool.AddTask([&sessions, &batches, thread_i, batch_i, &executed_batches, &password, promise = std::move(shared_promise)]() mutable {
          auto& batch = batches.at(batch_i);
          if (batch.backoff > 1) {
            std::cout << "executing batch: " << batch.index << " sleeping for: " << batch.backoff << "ms" << std::endl;
            std::this_thread::sleep_for(std::chrono::milliseconds(batch.backoff));
          }
          auto ret = query::ExecuteBatch(sessions[thread_i].get(), batch);
          if (ret.is_executed) {
            batch.is_executed = true;
            executed_batches++;
            std::cout << "batch: " << batch_i << " done" << std::endl;
            promise->Fill(true);
          } else {
            batch.backoff *= 2;
            if (batch.backoff > 100) {
              batch.backoff = 1;
            }
            batch.attempts += 1;
            promise->Fill(false);
          }
          // TODO(gitbuda): session can end up in a bad state -> debug and fix
          if (mg_session_status(sessions[thread_i].get()) == MG_SESSION_BAD) {
            sessions[thread_i] = MakeBoltSession(password);
          }
        });
        f_execs.insert_or_assign(thread_i, std::move(future));
      }
      int64_t no = used_threads;
      while (no > 0) {
        notifier.Await();
        --no;
      }

    }
    std::cout << executed_batches.load() << " batches done" << std::endl;
  }

  cleanup_resources();
  return 0;
}
