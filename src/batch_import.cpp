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
// TODO(gitbuda): If this is executed multiple times -> bad session + unknown message type -> debug
// TODO(gitbuda): The biggest issue seems to be that a few conflicting batches can end up in constant serialization
// conflict.
// TODO(gitbuda): Indexes are a problem because they can't be created in a multi-query transaction.
// TODO(gitbuda): Inject proper batch and query indexes.

struct Batches {
  Batches() = delete;
  Batches(const Batches &) = delete;
  Batches &operator=(const Batches &) = delete;
  Batches(Batches &&) = default;
  Batches &operator=(Batches &&) = default;

  explicit Batches(uint64_t max_size) {
    pure_vertices.reserve(max_size);
    others.reserve(max_size);
  }
  bool Empty() const { return pure_vertices.empty() && others.empty(); }

  std::vector<query::Batch> pure_vertices;
  std::vector<query::Batch> others;
};

struct BatchExecutionContext {
  BatchExecutionContext() = delete;
  BatchExecutionContext(const BatchExecutionContext &) = delete;
  BatchExecutionContext &operator=(const BatchExecutionContext &) = delete;
  BatchExecutionContext(BatchExecutionContext &&) = delete;
  BatchExecutionContext &operator=(BatchExecutionContext &&) = delete;

  BatchExecutionContext(uint64_t batch_size, uint64_t max_batches, uint64_t max_concurrent_executions,
                        const utils::bolt::Config &bolt_config)
      : batch_size(batch_size),
        max_batches(max_batches),
        max_concurrent_executions(max_concurrent_executions),
        thread_pool(max_concurrent_executions) {
    sessions.reserve(max_concurrent_executions);
    for (uint64_t thread_i = 0; thread_i < max_concurrent_executions; ++thread_i) {
      sessions[thread_i] = MakeBoltSession(bolt_config);
      // TODO(gitbuda): Handle failed connections required for the thread pool.
      if (!sessions[thread_i].get()) {
        std::cout << "a session uninitialized" << std::endl;
      }
    }
  }

  uint64_t batch_size;
  uint64_t max_batches;
  uint64_t max_concurrent_executions;
  utils::ThreadPool thread_pool{max_concurrent_executions};
  utils::Notifier notifier;
  std::vector<mg_memory::MgSessionPtr> sessions;
};

Batches FetchBatches(BatchExecutionContext &execution_context) {
  // TODO(gitbuda): Query index in FetchBatches is misleading -> rename.
  uint64_t query_index = 0;
  uint64_t batch_index = 0;
  query::Batch batch(execution_context.batch_size, batch_index);
  Batches batches(execution_context.max_batches);
  while (true) {
    if (query_index + 1 >= execution_context.batch_size * execution_context.max_batches) {
      break;
    }
    auto query = query::GetQuery(nullptr, true);
    if (!query) {
      break;
    }
    if (query->query.empty()) {
      continue;
    }
    query_index += 1;
    // TODO(gitbuda): Any processing Batch::apply(query) logic could be moved into the batch.
    auto updata_batch_type = [](const query::Query &query, query::Batch &batch) {
      batch.has_pure_nodes =
          batch.has_pure_nodes || (query.info->has_create && !query.info->has_match && !query.info->has_merge);
    };
    if (batch.queries.size() < execution_context.batch_size) {
      // TODO(gitbuda): Duplicated "batch type" logic -> centralize.
      updata_batch_type(*query, batch);
      batch.queries.emplace_back(std::move(*query));
    } else {
      batch_index += 1;
      // TODO(gitbuda): Duplicated batches routing logic -> centralize.
      if (batch.has_pure_nodes) {
        batches.pure_vertices.emplace_back(std::move(batch));
      } else {
        batches.others.emplace_back(std::move(batch));
      }
      batch = query::Batch(execution_context.batch_size, batch_index);
      // TODO(gitbuda): Duplicated "batch type" logic -> centralize.
      updata_batch_type(*query, batch);
      batch.queries.emplace_back(std::move(*query));
    }
  }
  // Add last batch if it's missing!
  if (batch.queries.size() > 0 && batch.queries.size() < execution_context.batch_size) {
    // TODO(gitbuda): Duplicated batches routing logic -> centralize.
    if (batch.has_pure_nodes) {
      batches.pure_vertices.emplace_back(std::move(batch));
    } else {
      batches.others.emplace_back(std::move(batch));
    }
  }
  return batches;
}

// TODO(gitbuda): Implement ExecuteBatches function.
/// returns the number of executed batches.
uint64_t ExecuteBatchesParallel(std::vector<query::Batch> &batches, BatchExecutionContext &execution_context,
                                const utils::bolt::Config &bolt_config) {
  if (batches.empty()) return 0;
  std::atomic<uint64_t> executed_batches = 0;
  while (true) {
    std::cout << "EXECUTED BATCHES: " << executed_batches << std::endl;
    if (executed_batches.load() >= batches.size()) {
      break;
    }

    std::unordered_map<size_t, utils::Future<bool>> f_execs;
    uint64_t used_threads = 0;
    for (uint64_t batch_i = 0; batch_i < batches.size(); ++batch_i) {
      if (used_threads >= execution_context.max_concurrent_executions) {
        break;
      }
      auto &batch = batches.at(batch_i);
      if (batch.is_executed) {
        continue;
      }

      // Schedule all batches for parallel execution.
      auto thread_i = used_threads;
      used_threads++;
      utils::ReadinessToken readiness_token{static_cast<size_t>(batch_i)};
      std::function<void()> fill_notifier = [readiness_token, &execution_context]() {
        execution_context.notifier.Notify(readiness_token);
      };
      auto [future, promise] = utils::FuturePromisePairWithNotifications<bool>(nullptr, fill_notifier);
      auto shared_promise = std::make_shared<decltype(promise)>(std::move(promise));
      execution_context.thread_pool.AddTask([&execution_context, &batches, thread_i, batch_i, &executed_batches,
                                             &bolt_config, promise = std::move(shared_promise)]() mutable {
        auto &batch = batches.at(batch_i);
        if (batch.backoff > 1) {
          // TODO(gitbuda): Replace with proper logging.
          std::cout << "executing batch: " << batch.index << " sleeping for: " << batch.backoff << "ms" << std::endl;
          std::this_thread::sleep_for(std::chrono::milliseconds(batch.backoff));
        }
        auto ret = query::ExecuteBatch(execution_context.sessions[thread_i].get(), batch);
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
        if (mg_session_status(execution_context.sessions[thread_i].get()) == MG_SESSION_BAD) {
          execution_context.sessions[thread_i] = MakeBoltSession(bolt_config);
        }
      });
      f_execs.insert_or_assign(thread_i, std::move(future));
    }

    // Wait for the execution to finish.
    int64_t no = used_threads;
    while (no > 0) {
      execution_context.notifier.Await();
      --no;
    }
  }
  return executed_batches.load();
}

int Run(const utils::bolt::Config &bolt_config) {
  BatchExecutionContext execution_context(100, 20, 16, bolt_config);
  std::random_device rd;
  std::default_random_engine rng(rd());
  std::uniform_int_distribution<> dist(2, 10);
  // TODO(gitbuda): shuffle is not suitable for analytics mode.
  // dist(rng) // to generate random int
  // std::shuffle(batches.begin(), batches.end(), rng);

  while (true) {
    auto batches = FetchBatches(execution_context);
    if (batches.Empty()) {
      break;
    }
    auto pure_vertices_batch_executed = ExecuteBatchesParallel(batches.pure_vertices, execution_context, bolt_config);
    auto others_batch_executed = ExecuteBatchesParallel(batches.others, execution_context, bolt_config);
    std::cout << pure_vertices_batch_executed + others_batch_executed << " batches executed" << std::endl;
  }
  return 0;
}

}  // namespace mode::batch_import
