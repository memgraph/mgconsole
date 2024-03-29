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
#include <memory>
#include <mutex>
#include <optional>
#include <thread>
#include <utility>

#include "assert.hpp"

namespace utils {

// Shared is in an anonymous namespace, and the only way to
// construct a Promise or Future is to pass a Shared in. This
// ensures that Promises and Futures can only be constructed
// in this translation unit.
namespace details {
template <typename T>
class Shared {
  mutable std::condition_variable cv_;
  mutable std::mutex mu_;
  std::optional<T> item_;
  bool consumed_ = false;
  bool waiting_ = false;
  bool filled_ = false;
  std::function<bool()> wait_notifier_ = nullptr;
  std::function<void()> fill_notifier_ = nullptr;

 public:
  explicit Shared(std::function<bool()> wait_notifier, std::function<void()> fill_notifier)
      : wait_notifier_(wait_notifier), fill_notifier_(fill_notifier) {}
  Shared() = default;
  Shared(Shared &&) = delete;
  Shared &operator=(Shared &&) = delete;
  Shared(const Shared &) = delete;
  Shared &operator=(const Shared &) = delete;
  ~Shared() = default;

  /// Takes the item out of our optional item_ and returns it.
  T Take() {
    MG_ASSERT(item_, "Take called without item_ being present");
    MG_ASSERT(!consumed_, "Take called on already-consumed Future");

    T ret = std::move(item_).value();
    item_.reset();

    consumed_ = true;

    return ret;
  }

  T Wait() {
    std::unique_lock<std::mutex> lock(mu_);
    waiting_ = true;

    while (!item_) {
      if (wait_notifier_) [[unlikely]] {
        // We can't hold our own lock while notifying
        // the simulator because notifying the simulator
        // involves acquiring the simulator's mutex
        // to guarantee that our notification linearizes
        // with the simulator's condition variable.
        // However, the simulator may acquire our
        // mutex to check if we are being awaited,
        // while determining system quiescence,
        // so we have to get out of its way to avoid
        // a cyclical deadlock.
        lock.unlock();
        std::invoke(wait_notifier_);
        lock.lock();
        if (item_) {
          // item may have been filled while we
          // had dropped our mutex while notifying
          // the simulator of our waiting_ status.
          break;
        }
      } else {
        cv_.wait(lock);
      }
      MG_ASSERT(!consumed_, "Future consumed twice!");
    }

    waiting_ = false;

    return Take();
  }

  bool IsReady() const {
    std::unique_lock<std::mutex> lock(mu_);
    return item_.has_value();
  }

  std::optional<T> TryGet() {
    std::unique_lock<std::mutex> lock(mu_);

    if (item_) {
      return Take();
    }

    return std::nullopt;
  }

  void Fill(T item) {
    {
      std::unique_lock<std::mutex> lock(mu_);

      MG_ASSERT(!consumed_, "Promise filled after it was already consumed!");
      MG_ASSERT(!filled_, "Promise filled twice!");

      item_ = item;
      filled_ = true;
    }  // lock released before condition variable notification

    if (fill_notifier_) {
      std::invoke(fill_notifier_);
    }

    cv_.notify_all();
  }

  bool IsAwaited() const {
    std::unique_lock<std::mutex> lock(mu_);
    return waiting_;
  }
};
}  // namespace details

template <typename T>
class Future {
  bool consumed_or_moved_ = false;
  std::shared_ptr<details::Shared<T>> shared_;

 public:
  explicit Future(std::shared_ptr<details::Shared<T>> shared) : shared_(shared) {}

  Future() = delete;
  Future(Future &&old) noexcept {
    MG_ASSERT(!old.consumed_or_moved_, "Future moved from after already being moved from or consumed.");
    shared_ = std::move(old.shared_);
    consumed_or_moved_ = old.consumed_or_moved_;
    old.consumed_or_moved_ = true;
  }

  Future &operator=(Future &&old) noexcept {
    MG_ASSERT(!old.consumed_or_moved_, "Future moved from after already being moved from or consumed.");
    shared_ = std::move(old.shared_);
    consumed_or_moved_ = old.consumed_or_moved_;
    old.consumed_or_moved_ = true;
    return *this;
  }

  Future(const Future &) = delete;
  Future &operator=(const Future &) = delete;
  ~Future() = default;

  /// Returns true if the Future is ready to
  /// be consumed using TryGet or Wait (prefer Wait
  /// if you know it's ready, because it doesn't
  /// return an optional.
  bool IsReady() {
    MG_ASSERT(!consumed_or_moved_, "Called IsReady after Future already consumed!");
    return shared_->IsReady();
  }

  /// Non-blocking method that returns the inner
  /// item if it's already ready, or std::nullopt
  /// if it is not ready yet.
  std::optional<T> TryGet() {
    MG_ASSERT(!consumed_or_moved_, "Called TryGet after Future already consumed!");
    std::optional<T> ret = shared_->TryGet();
    if (ret) {
      consumed_or_moved_ = true;
    }
    return ret;
  }

  /// Block on the corresponding promise to be filled,
  /// returning the inner item when ready.
  T Wait() && {
    MG_ASSERT(!consumed_or_moved_, "Future should only be consumed with Wait once!");
    T ret = shared_->Wait();
    consumed_or_moved_ = true;
    return ret;
  }

  /// Marks this Future as canceled.
  void Cancel() {
    MG_ASSERT(!consumed_or_moved_, "Future::Cancel called on a future that was already moved or consumed!");
    consumed_or_moved_ = true;
  }
};

template <typename T>
class Promise {
  std::shared_ptr<details::Shared<T>> shared_;
  bool filled_or_moved_{false};

 public:
  explicit Promise(std::shared_ptr<details::Shared<T>> shared) : shared_(shared) {}

  Promise() = delete;
  Promise(Promise &&old) noexcept {
    MG_ASSERT(!old.filled_or_moved_, "Promise moved from after already being moved from or filled.");
    shared_ = std::move(old.shared_);
    old.filled_or_moved_ = true;
  }

  Promise &operator=(Promise &&old) noexcept {
    MG_ASSERT(!old.filled_or_moved_, "Promise moved from after already being moved from or filled.");
    shared_ = std::move(old.shared_);
    old.filled_or_moved_ = true;
  }
  Promise(const Promise &) = delete;
  Promise &operator=(const Promise &) = delete;

  // NOLINTNEXTLINE(clang-analyzer-core.uninitialized.Branch)
  ~Promise() { MG_ASSERT(filled_or_moved_, "Promise destroyed before its associated Future was filled!"); }

  // Fill the expected item into the Future.
  void Fill(T item) {
    MG_ASSERT(!filled_or_moved_, "Promise::Fill called on a promise that is already filled or moved!");
    shared_->Fill(item);
    filled_or_moved_ = true;
  }

  bool IsAwaited() { return shared_->IsAwaited(); }

  /// Moves this Promise into a unique_ptr.
  std::unique_ptr<Promise<T>> ToUnique() && {
    std::unique_ptr<Promise<T>> up = std::make_unique<Promise<T>>(std::move(shared_));

    filled_or_moved_ = true;

    return up;
  }
};

template <typename T>
std::pair<Future<T>, Promise<T>> FuturePromisePair() {
  std::shared_ptr<details::Shared<T>> shared = std::make_shared<details::Shared<T>>();

  Future<T> future = Future<T>(shared);
  Promise<T> promise = Promise<T>(shared);

  return std::make_pair(std::move(future), std::move(promise));
}

template <typename T>
std::pair<Future<T>, Promise<T>> FuturePromisePairWithNotifications(std::function<bool()> wait_notifier,
                                                                    std::function<void()> fill_notifier) {
  std::shared_ptr<details::Shared<T>> shared = std::make_shared<details::Shared<T>>(wait_notifier, fill_notifier);

  Future<T> future = Future<T>(shared);
  Promise<T> promise = Promise<T>(shared);

  return std::make_pair(std::move(future), std::move(promise));
}

}  // namespace utils
