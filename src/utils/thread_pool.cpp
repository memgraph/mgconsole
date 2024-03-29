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

#include "thread_pool.hpp"

namespace utils {

ThreadPool::ThreadPool(const size_t pool_size) {
  for (size_t i = 0; i < pool_size; ++i) {
    thread_pool_.emplace_back(([this] { this->ThreadLoop(); }));
  }
}

void ThreadPool::AddTask(std::function<void()> new_task) {
  task_queue_.WithLock([&](auto &queue) {
    queue.emplace(std::make_unique<TaskSignature>(std::move(new_task)));
    unfinished_tasks_num_.fetch_add(1);
  });
  std::unique_lock pool_guard(pool_lock_);
  queue_cv_.notify_one();
}

void ThreadPool::Shutdown() {
  terminate_pool_.store(true);
  {
    std::unique_lock pool_guard(pool_lock_);
    queue_cv_.notify_all();
  }

  for (auto &thread : thread_pool_) {
    if (thread.joinable()) {
      thread.join();
    }
  }

  thread_pool_.clear();
  stopped_.store(true);
}

ThreadPool::~ThreadPool() {
  if (!stopped_.load()) {
    Shutdown();
  }
}

std::unique_ptr<ThreadPool::TaskSignature> ThreadPool::PopTask() {
  return task_queue_.WithLock([](auto &queue) -> std::unique_ptr<TaskSignature> {
    if (queue.empty()) {
      return nullptr;
    }
    auto front = std::move(queue.front());
    queue.pop();
    return front;
  });
}

void ThreadPool::ThreadLoop() {
  std::unique_ptr<TaskSignature> task = PopTask();
  while (true) {
    while (task) {
      if (terminate_pool_.load()) {
        return;
      }
      (*task)();
      unfinished_tasks_num_.fetch_sub(1);
      task = PopTask();
    }

    std::unique_lock guard(pool_lock_);
    queue_cv_.wait(guard, [&] {
      task = PopTask();
      return task || terminate_pool_.load();
    });
    if (terminate_pool_.load()) {
      return;
    }
  }
}

size_t ThreadPool::UnfinishedTasksNum() const { return unfinished_tasks_num_.load(); }

}  // namespace utils
