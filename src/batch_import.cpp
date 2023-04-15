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

#include "batch_import.hpp"

#include <random>
#include <thread>
#include <unordered_map>

#include <gflags/gflags.h>

#include "utils/bolt.hpp"
#include "utils/constants.hpp"
#include "utils/future.hpp"
#include "utils/notifier.hpp"
#include "utils/thread_pool.hpp"
#include "utils/utils.hpp"

namespace mode::batch_import {

using namespace std::string_literals;

// NOTE: A big problem with batched execution is that executing batches with
//       edges will pass on an empty database (with no nodes) without an error:
//         * edges have to come after nodes
//         * count how many elements is actually created from a given batch
//
// TODO(gitbuda): All queries have to be stored or at least N * batch_size
// TODO(gitbuda): If this is executed multiple times -> bad session + unknown message type -> debug
// TODO(gitbuda): The biggest issue seems to be that a few conflicting batches can end up in constant serialization
// conflict.
// TODO(gitbuda): Figure out the right batched and parallel execution.
// TODO(gitbuda): Figure out how to print batch execution result.
// TODO(gitbuda): Indexes are a problem because they can't be created in a multi-query transaction.
// TODO(gitbuda): Inject proper batch and query indexes.

std::vector<query::Batch> FetchBatches(uint64_t batch_size, uint64_t max_batches) {
  uint64_t query_index = 0;
  uint64_t batch_index = 0;

  query::Batch batch(batch_size, batch_index);
  std::vector<query::Batch> batches;
  batches.reserve(max_batches);

  while (true) {
    if (query_index + 1 >= batch_size * max_batches) {
      break;
    }
    auto query = query::GetQuery(nullptr);
    if (!query) {
      break;
    }
    if (query->empty()) {
      continue;
    }
    query_index += 1;

    if (batch.queries.size() < batch_size) {
      batch.queries.emplace_back(query::Query{.line_number = 0, .index = 0, .query = *query});
    } else {
      batch_index += 1;
      batches.emplace_back(std::move(batch));
      batch = query::Batch(batch_size, batch_index);
      batch.queries.emplace_back(query::Query{.line_number = 0, .index = 0, .query = *query});
    }
  }
  // Add last batch if it's missing!
  if (batch.queries.size() > 0 && batch.queries.size() < batch_size) {
    batches.emplace_back(std::move(batch));
  }

  return batches;
}

int Run(const utils::bolt::Config &bolt_config) {
  uint64_t batch_size = 100;
  uint64_t max_batches = 20;
  int64_t max_concurrent_executions = 16;
  uint64_t thread_pool_size = 16;

  utils::ThreadPool thread_pool(thread_pool_size);
  utils::Notifier notifier;
  std::vector<mg_memory::MgSessionPtr> sessions;
  sessions.reserve(thread_pool_size);
  for (uint64_t thread_i = 0; thread_i < thread_pool_size; ++thread_i) {
    sessions[thread_i] = MakeBoltSession(bolt_config);
    // TODO(gitbuda): Handle failed connections required for the thread pool.
    if (!sessions[thread_i].get()) {
      std::cout << "a session uninitialized" << std::endl;
    }
  }
  std::random_device rd;
  std::default_random_engine rng(rd());
  std::uniform_int_distribution<> dist(2, 10);
  // dist(rng) // to generate random int

  while (true) {
    auto batches = FetchBatches(batch_size, max_batches);
    if (batches.empty()) {
      break;
    }
    std::shuffle(batches.begin(), batches.end(), rng);

    std::atomic<uint64_t> executed_batches = 0;
    while (true) {
      std::cout << "EXECUTED BATCHES: " << executed_batches << std::endl;
      if (executed_batches.load() >= batches.size()) {
        break;
      }

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
        thread_pool.AddTask([&sessions, &batches, thread_i, batch_i, &executed_batches, &bolt_config,
                             promise = std::move(shared_promise)]() mutable {
          auto &batch = batches.at(batch_i);
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
            sessions[thread_i] = MakeBoltSession(bolt_config);
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
    std::cout << executed_batches.load() << " batches executed" << std::endl;
  }
  return 0;
}

}  // namespace mode::batch_import
