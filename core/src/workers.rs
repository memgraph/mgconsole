//! Worker Sessions for batched-parallel import (slice 29).
//!
//! Per ADR 0006 this is a **worker-pull** model, not a connection-pool crate: a
//! Session is a single stream, so N concurrent workers need N long-lived
//! connections. [`Workers`] establishes them up front with the same
//! host/port/auth/TLS config as a single Session; slice 30 spawns a task per
//! Session, each pulling Batches from a shared queue.
//!
//! Each worker owns an independent [`Session`], so a worker whose connection
//! breaks reconnects through that Session's own bounded reconnect (slice 14)
//! without disturbing the others — there is no pool replacing connections.

use crate::error::Error;
use crate::session::{ConnectOptions, Session};

/// A fixed set of long-lived worker Sessions, each its own Bolt connection.
pub struct Workers {
    sessions: Vec<Session>,
}

impl Workers {
    /// Establish `count` worker Sessions against `host:port`, each authenticating
    /// with the same `options` as a single Session. `count` of 0 auto-detects
    /// from available parallelism (the `--workers-number 0` default).
    ///
    /// If any worker fails to connect, the error surfaces and the Sessions opened
    /// so far are dropped (closing their connections).
    pub async fn connect(
        host: &str,
        port: u16,
        options: &ConnectOptions,
        count: usize,
    ) -> Result<Self, Error> {
        let count = resolve_count(count);
        let mut sessions = Vec::with_capacity(count);
        for _ in 0..count {
            sessions.push(Session::connect_with(host, port, options).await?);
        }
        Ok(Self { sessions })
    }

    /// Wrap already-established Sessions as workers (used by tests that route each
    /// worker through its own transport, and available for advanced wiring).
    pub fn from_sessions(sessions: Vec<Session>) -> Self {
        Self { sessions }
    }

    /// The number of workers (bounds the number of live connections).
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether there are no workers.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Mutable access to the worker Sessions, e.g. to run a query on each.
    pub fn sessions_mut(&mut self) -> &mut [Session] {
        &mut self.sessions
    }

    /// Consume the workers, yielding the owned Sessions — slice 30 moves each into
    /// its own task.
    pub fn into_sessions(self) -> Vec<Session> {
        self.sessions
    }
}

/// Resolve a requested worker count: 0 means auto-detect from the machine's
/// available parallelism, falling back to 1 when that cannot be determined.
fn resolve_count(requested: usize) -> usize {
    if requested > 0 {
        return requested;
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_count_is_used_verbatim() {
        assert_eq!(resolve_count(4), 4);
        assert_eq!(resolve_count(1), 1);
    }

    #[test]
    fn zero_auto_detects_at_least_one() {
        assert!(resolve_count(0) >= 1);
    }
}
