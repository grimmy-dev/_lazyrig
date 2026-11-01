//! The activity signal: a start instant and two atomics. A beat costs one clock read and
//! one store; nothing wakes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct PulseState {
    /// The tick's start. Every beat is stored as an offset from it.
    start: Instant,
    /// Ms since `start`, plus 1. 0 means no beat yet, so a beat at 0 ms still counts.
    last_ms: AtomicU64,
    /// Live `Hold` guards.
    holds: AtomicU32,
}

/// The activity signal of one running task. Clone it into whatever reads the work's
/// output; every clone shares one state.
#[derive(Clone, Debug)]
pub struct Pulse(Arc<PulseState>);

/// While a `Hold` lives, the stale timer does not fire. Its drop is a beat. Not `Clone`:
/// a copy would release twice.
#[derive(Debug)]
#[must_use = "a Hold releases when dropped; bind it with `let _hold`"]
pub struct Hold(Pulse);

impl Pulse {
    /// A pulse with no beat and no hold. `start` is the tick's start.
    #[must_use]
    pub fn new(start: Instant) -> Self {
        Self(Arc::new(PulseState {
            start,
            last_ms: AtomicU64::new(0),
            holds: AtomicU32::new(0),
        }))
    }

    /// Record activity now. One clock read and one store, no lock.
    pub fn beat(&self) {
        self.beat_at(Instant::now());
    }

    /// Record activity at `at`: the pure core of `beat`, so a test can pass a made-up
    /// instant. `at` must not be in the future, or the task looks alive longer than it is.
    pub fn beat_at(&self, at: Instant) {
        let ms = u64::try_from(at.saturating_duration_since(self.0.start).as_millis())
            .unwrap_or(u64::MAX);
        self.0
            .last_ms
            .store(ms.saturating_add(1), Ordering::Release);
    }

    /// When the last beat happened. `None` before the first beat.
    #[must_use]
    pub fn last_beat(&self) -> Option<Instant> {
        match self.0.last_ms.load(Ordering::Relaxed) {
            0 => None,
            n => Some(self.0.start + Duration::from_millis(n - 1)),
        }
    }

    /// Pause the stale timer until the returned guard drops. The hard timer keeps running.
    pub fn hold(&self) -> Hold {
        self.0.holds.fetch_add(1, Ordering::Relaxed);
        Hold(self.clone())
    }

    /// Whether any `Hold` is alive. Read it before `last_beat`: the `Acquire` here makes
    /// the beat of a released hold visible.
    #[must_use]
    pub fn held(&self) -> bool {
        self.0.holds.load(Ordering::Acquire) > 0
    }

    /// The tick's start.
    #[must_use]
    pub fn start(&self) -> Instant {
        self.0.start
    }
}

impl Drop for Hold {
    /// Beat first, then release. A reader that sees the release also sees the beat, so it
    /// never judges the task by a beat from before the hold.
    fn drop(&mut self) {
        self.0.beat();
        self.0.0.holds.fetch_sub(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    const WAIT: Duration = Duration::from_millis(5);

    #[test]
    fn beat_records_the_time() {
        let pulse = Pulse::new(Instant::now());
        assert_eq!(pulse.last_beat(), None);
        sleep(WAIT);
        pulse.beat();
        let last = pulse.last_beat().unwrap();
        assert!(last >= pulse.start() + WAIT);
    }

    #[test]
    fn held_until_the_last_hold_drops() {
        let pulse = Pulse::new(Instant::now());
        let first = pulse.hold();
        let second = pulse.hold();
        assert!(pulse.held());
        drop(first);
        assert!(pulse.held());
        drop(second);
        assert!(!pulse.held());
        assert!(pulse.last_beat().is_some());
    }

    #[test]
    fn clones_share_one_state() {
        let pulse = Pulse::new(Instant::now());
        let reader = pulse.clone();
        reader.beat();
        assert!(pulse.last_beat().is_some());
    }
}
