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

#pragma once

#include <atomic>
#include <condition_variable>
#include <functional>
#include <mutex>
#include <queue>
#include <thread>

#include "synchronized.hpp"

namespace utils {

class ThreadPool {
  using TaskSignature = std::function<void()>;

 public:
  explicit ThreadPool(size_t pool_size);
  ThreadPool(const ThreadPool &) = delete;
  ThreadPool(ThreadPool &&) = delete;
  ThreadPool &operator=(const ThreadPool &) = delete;
  ThreadPool &operator=(ThreadPool &&) = delete;
  ~ThreadPool();

  void AddTask(std::function<void()> new_task);
  size_t UnfinishedTasksNum() const;
  void Shutdown();

 private:
  void ThreadLoop();
  std::unique_ptr<TaskSignature> PopTask();

  std::vector<std::thread> thread_pool_;
  std::atomic<size_t> unfinished_tasks_num_{0};
  std::atomic<bool> terminate_pool_{false};
  std::atomic<bool> stopped_{false};
  utils::Synchronized<std::queue<std::unique_ptr<TaskSignature>>, std::mutex> task_queue_;
  std::mutex pool_lock_;
  std::condition_variable queue_cv_;
};

}  // namespace utils
