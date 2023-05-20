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

#include <condition_variable>
#include <functional>
#include <mutex>
#include <optional>
#include <vector>

namespace utils {

class ReadinessToken {
  size_t id_;

 public:
  explicit ReadinessToken(size_t id) : id_(id) {}
  size_t GetId() const { return id_; }
};

class Inner {
  std::condition_variable cv_;
  std::mutex mu_;
  std::vector<ReadinessToken> ready_;
  std::optional<std::function<bool()>> tick_simulator_;

 public:
  void Notify(ReadinessToken readiness_token) {
    {
      std::unique_lock<std::mutex> lock(mu_);
      ready_.emplace_back(readiness_token);
    }  // mutex dropped

    cv_.notify_all();
  }

  ReadinessToken Await() {
    std::unique_lock<std::mutex> lock(mu_);

    while (ready_.empty()) {
      if (tick_simulator_) [[unlikely]] {
        // This avoids a deadlock in a similar way that
        // Future::Wait will release its mutex while
        // interacting with the simulator, due to
        // the fact that the simulator may cause
        // notifications that we are interested in.
        lock.unlock();
        std::invoke(tick_simulator_.value());
        lock.lock();
      } else {
        cv_.wait(lock);
      }
    }

    ReadinessToken ret = ready_.back();
    ready_.pop_back();
    return ret;
  }

  void InstallSimulatorTicker(std::function<bool()> tick_simulator) {
    std::unique_lock<std::mutex> lock(mu_);
    tick_simulator_ = tick_simulator;
  }
};

class Notifier {
  std::shared_ptr<Inner> inner_;

 public:
  Notifier() : inner_(std::make_shared<Inner>()) {}
  Notifier(const Notifier &) = default;
  Notifier &operator=(const Notifier &) = default;
  Notifier(Notifier &&old) = default;
  Notifier &operator=(Notifier &&old) = default;
  ~Notifier() = default;

  void Notify(ReadinessToken readiness_token) const { inner_->Notify(readiness_token); }

  ReadinessToken Await() const { return inner_->Await(); }

  void InstallSimulatorTicker(std::function<bool()> tick_simulator) { inner_->InstallSimulatorTicker(tick_simulator); }
};

}  // namespace utils
