//! One watched tick: its timer state, shared with the reaper, and the handle the task
//! holds.

use crate::Ticker;
use crate::budget::budget;
use crate::reaper::Cmd;
use lazyrig_core::pulse::Pulse;
use lazyrig_core::time::{Budget, Expiry, TickId, WatchKind};
use lazyrig_ports::Watch;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// The longest budget a timer keeps; a larger one (a lua typo, a huge model timeout) is cut
/// to this when the timer is armed. So the deadline math in `check` never overflows, and
/// every deadline stays far inside tokio's timer range (about 2 years).
pub(crate) const MAX_BUDGET: Duration = Duration::from_hours(30 * 24);

/// One armed timer. Budgets are ms after the tick's start, so they fit in atomics:
/// `reclassify` changes them while the reaper reads.
pub(crate) struct Timer {
    /// The task's activity signal.
    pub pulse: Pulse,
    /// The hard timer. `u64::MAX` = no ceiling.
    pub hard_ms: AtomicU64,
    /// The stale timer: the longest gap with no beat.
    pub idle_ms: AtomicU64,
    /// Quiet allowed after the start before the stale timer counts.
    pub grace_ms: u64,
    /// The task's own token. The reaper fires it; the task does the kill.
    pub cancel: CancellationToken,
    /// Why the token fired. Set once, by the reaper only.
    pub expired: OnceLock<Expiry>,
}

impl Timer {
    /// Arm a timer. Every budget is clamped to `MAX_BUDGET` here, once, so no reader
    /// checks it again.
    pub fn new(pulse: Pulse, budget: Budget, grace: Duration, cancel: CancellationToken) -> Self {
        Self {
            pulse,
            hard_ms: AtomicU64::new(budget.timeout.map_or(u64::MAX, clamped_ms)),
            idle_ms: AtomicU64::new(clamped_ms(budget.idle_timeout)),
            grace_ms: clamped_ms(grace),
            cancel,
            expired: OnceLock::new(),
        }
    }

    /// Swap in new budgets, clamped like `new`. The reaper may read one new and one old
    /// value once; its next check sees both.
    pub fn set_budget(&self, budget: Budget) {
        self.hard_ms.store(
            budget.timeout.map_or(u64::MAX, clamped_ms),
            Ordering::Relaxed,
        );
        self.idle_ms
            .store(clamped_ms(budget.idle_timeout), Ordering::Relaxed);
    }
}

/// A budget in ms, clamped to `MAX_BUDGET`. The result is far below `u64::MAX`, so a
/// clamped hard timer never reads as "no ceiling".
fn clamped_ms(d: Duration) -> u64 {
    u64::try_from(d.min(MAX_BUDGET).as_millis()).unwrap_or(u64::MAX)
}

/// The handle a watched task holds. `Drop` is the cleanup; `finish` is the same plus the
/// elapsed.
pub(crate) struct TickWatch {
    pub id: TickId,
    pub start: Instant,
    pub timer: Arc<Timer>,
    /// The model's ceiling, kept so `reclassify` applies it to the new kind.
    pub model_timeout: Option<Duration>,
    /// Keeps the reaper's channel open while the watch lives.
    pub ticker: Ticker,
}

impl Watch for TickWatch {
    fn id(&self) -> TickId {
        self.id
    }

    fn pulse(&self) -> Pulse {
        self.timer.pulse.clone()
    }

    fn expired(&self) -> Option<Expiry> {
        self.timer.expired.get().copied()
    }

    fn reclassify(&mut self, kind: WatchKind) {
        // No command to the reaper: at the old deadline `check` reads the new budget and
        // waits again (lazy re-arm). A tighter budget therefore waits for the old deadline.
        let timeouts = (self.ticker.0.timeouts)();
        self.timer
            .set_budget(budget(&timeouts, kind, self.model_timeout));
        self.ticker.0.table.set_kind(self.id, kind.into());
    }

    fn finish(self: Box<Self>) -> Duration {
        let took = self.start.elapsed();
        // `Drop` removes the tick and disarms.
        drop(self);
        took
    }
}

impl Drop for TickWatch {
    fn drop(&mut self) {
        // Idempotent: after a fire, both calls miss and do nothing.
        let _ = self.ticker.0.table.remove(self.id);
        // Fails only when the reaper is gone, and then nothing is armed.
        let _ = self.ticker.0.tx.send(Cmd::Disarm(self.id));
    }
}
