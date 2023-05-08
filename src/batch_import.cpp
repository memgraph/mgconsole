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

struct Batches {
  Batches() = delete;
  Batches(const Batches &) = delete;
  Batches &operator=(const Batches &) = delete;
  Batches(Batches &&) = default;
  Batches &operator=(Batches &&) = default;

  explicit Batches(uint64_t batch_size, uint64_t max_batches)
      : batch_size(batch_size), pure_vertices_batch(batch_size, 0), others_batch(batch_size, 1) {
    batch_index = 1;
    pure_vertices.reserve(max_batches);
    others.reserve(max_batches);
  }
  bool Empty() const { return pure_vertices.empty() && others.empty(); }

  void AddQuery(query::Query query) {
    auto is_pure_vertex_query = [](const query::Query &query) {
      return query.info->has_create && !query.info->has_match && !query.info->has_merge;
    };
    if (is_pure_vertex_query(query)) {
      if (pure_vertices_batch.queries.size() < batch_size) {
        pure_vertices_batch.queries.emplace_back(std::move(query));
      } else {
        batch_index += 1;
        pure_vertices.emplace_back(std::move(pure_vertices_batch));
        pure_vertices_batch = query::Batch(batch_size, batch_index);
        pure_vertices_batch.queries.emplace_back(std::move(query));
      }
    } else {
      if (others_batch.queries.size() < batch_size) {
        others_batch.queries.emplace_back(std::move(query));
      } else {
        batch_index += 1;
        others.emplace_back(std::move(others_batch));
        others_batch = query::Batch(batch_size, batch_index);
        others_batch.queries.emplace_back(std::move(query));
      }
    }
  }

  // Add last batch if it's missing!
  void Finalize() {
    if (pure_vertices_batch.queries.size() > 0 && pure_vertices_batch.queries.size() < batch_size) {
      pure_vertices.emplace_back(std::move(pure_vertices_batch));
    }
    if (others_batch.queries.size() > 0 && others_batch.queries.size() < batch_size) {
      others.emplace_back(std::move(others_batch));
    }
  }

  uint64_t VertexQueryNo() const {
    uint64_t no = 0;
    for (const auto &b : pure_vertices) {
      no += b.queries.size();
    }
    return no;
  }
  uint64_t OthersNo() const {
    uint64_t no = 0;
    for (const auto &b : others) {
      no += b.queries.size();
    }
    return no;
  }
  uint64_t TotalQueryNo() const { return VertexQueryNo() + OthersNo(); }

  uint64_t batch_size;
  uint64_t batch_index{0};
  query::Batch pure_vertices_batch;
  query::Batch others_batch;
  std::vector<query::Batch> pure_vertices;
  std::vector<query::Batch> others;
};

inline std::ostream &operator<<(std::ostream &os, const Batches &bs) {
  os << "Batches .pure_vertices " << bs.pure_vertices.size() << " .others " << bs.others.size() << '\n';
  os << "  pure_vertices" << '\n';
  for (const auto &b : bs.pure_vertices) {
    os << "  " << b.queries.size() << '\n';
  }
  os << "  others" << '\n';
  for (const auto &b : bs.others) {
    os << "  " << b.queries.size() << '\n';
  }
  return os;
}

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
  Batches batches(execution_context.batch_size, execution_context.max_batches);
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
    batches.AddQuery(std::move(*query));
  }
  batches.Finalize();
  return batches;
}

/// returns the number of executed batches.
uint64_t ExecuteBatchesParallel(std::vector<query::Batch> &batches, BatchExecutionContext &execution_context,
                                const utils::bolt::Config &bolt_config) {
  if (batches.empty()) return 0;
  std::atomic<uint64_t> executed_batches = 0;
  while (true) {
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
          std::this_thread::sleep_for(std::chrono::milliseconds(batch.backoff));
        }
        auto ret = query::ExecuteBatch(execution_context.sessions[thread_i].get(), batch);
        if (ret.is_executed) {
          batch.is_executed = true;
          executed_batches++;
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
  while (true) {
    auto batches = FetchBatches(execution_context);
    if (batches.Empty()) {
      break;
    }
    ExecuteBatchesParallel(batches.pure_vertices, execution_context, bolt_config);
    ExecuteBatchesParallel(batches.others, execution_context, bolt_config);
  }
  return 0;
}

}  // namespace mode::batch_import
