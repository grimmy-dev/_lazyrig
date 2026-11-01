//! The reaper: one task that fires spent timers. `check` is the pure rule; `run` is the
//! loop around it on tokio's timer wheel.

use crate::table::Table;
use crate::watch::Timer;
use lazyrig_core::time::{Expiry, TickId};
use std::collections::HashMap;
use std::future::poll_fn;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::time::DelayQueue;
use tokio_util::time::delay_queue::Key;

/// The shortest re-check while a timer is held. A zero idle budget would otherwise make
/// `check` wait until `now`, and the reaper would spin for as long as the hold lives.
const MIN_HELD_WAIT: Duration = Duration::from_secs(1);

/// What the rule decided.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Check {
    /// A timer is spent: fire the token now.
    Fire(Expiry),
    /// Nothing is spent: look again at this instant.
    Wait(Instant),
}

/// Fire now, or wait until an instant. Pure: no clock read, no lock, so a test passes any
/// `now`. The hard timer wins over the stale one; a hold pauses the stale timer only.
///
/// No overflow check: `Timer::new` clamps every budget to `MAX_BUDGET`.
pub(crate) fn check(now: Instant, timer: &Timer) -> Check {
    let start = timer.pulse.start();

    // Hard timer: an absolute ceiling, counted from the start.
    let hard = match timer.hard_ms.load(Ordering::Relaxed) {
        u64::MAX => None,
        ms => Some(start + Duration::from_millis(ms)),
    };
    if let Some(hard) = hard
        && now >= hard
    {
        return Check::Fire(Expiry::Timeout);
    }

    // Stale timer. Read `held` before `last_beat`: its Acquire makes the beat of a
    // released hold visible (see `Hold::drop`).
    let idle = Duration::from_millis(timer.idle_ms.load(Ordering::Relaxed));
    let stale = if timer.pulse.held() {
        // Held: never stale, but look again one idle window from now, at least
        // `MIN_HELD_WAIT` away.
        now + idle.max(MIN_HELD_WAIT)
    } else {
        // Silence counts from the last beat, never before the grace ends.
        let grace_end = start + Duration::from_millis(timer.grace_ms);
        let last = timer
            .pulse
            .last_beat()
            .map_or(grace_end, |beat| beat.max(grace_end));
        let stale = last + idle;
        if now >= stale {
            return Check::Fire(Expiry::Stale);
        }
        stale
    };

    // Both deadlines are still ahead: wake at the earlier one.
    Check::Wait(hard.map_or(stale, |hard| hard.min(stale)))
}

/// What a watch tells the reaper. Sent on an unbounded channel, so `watch` and a drop
/// never block and never await.
pub(crate) enum Cmd {
    /// Start timing this tick.
    Arm(TickId, Arc<Timer>),
    /// Forget this tick without firing: the task ended by itself.
    Disarm(TickId),
}

/// The reaper task. Sleeps until the earliest deadline or a command, so an idle process
/// costs zero wakes. Returns when every sender is gone.
pub(crate) async fn run(mut rx: UnboundedReceiver<Cmd>, table: Arc<Table>) {
    // The queue holds only ids; `keys` maps each id to its queue key and its timer.
    let mut queue: DelayQueue<TickId> = DelayQueue::new();
    let mut keys: HashMap<TickId, (Key, Arc<Timer>)> = HashMap::new();

    loop {
        tokio::select! {
            // Commands first: a Disarm must win over a fire of the same tick.
            biased;

            cmd = rx.recv() => match cmd {
                // Every `Ticker` and every watch is gone: nothing can arm a timer again.
                None => return,
                Some(Cmd::Arm(id, timer)) => {
                    let key = queue.insert_at(id, due(Instant::now(), &timer));
                    keys.insert(id, (key, timer));
                }
                Some(Cmd::Disarm(id)) => {
                    // A miss means a fire got there first (G2). `try_remove` never panics.
                    if let Some((key, _)) = keys.remove(&id) {
                        queue.try_remove(&key);
                    }
                }
            },

            // The guard matters: an empty queue is ready at once with `None`, and the loop
            // would spin at 100% CPU.
            Some(entry) = poll_fn(|cx| queue.poll_expired(cx)), if !queue.is_empty() => {
                let id = entry.into_inner();
                // The expired key is dead, so take the timer out; a `Wait` puts it back.
                let Some((_, timer)) = keys.remove(&id) else {
                    continue;
                };
                match check(Instant::now(), &timer) {
                    // Activity moved the deadline: arm again, lazily (D2).
                    Check::Wait(at) => {
                        let key = queue.insert_at(id, tokio::time::Instant::from_std(at));
                        keys.insert(id, (key, timer));
                    }
                    Check::Fire(expiry) => fire(&table, id, &timer, expiry),
                }
            }
        }
    }
}

/// When to look at a timer next. A spent one is due now.
fn due(now: Instant, timer: &Timer) -> tokio::time::Instant {
    let at = match check(now, timer) {
        Check::Fire(_) => now,
        Check::Wait(at) => at,
    };
    tokio::time::Instant::from_std(at)
}

/// Record why, fire the token, drop the tick (D4). The task kills its own work.
fn fire(table: &Table, id: TickId, timer: &Timer, expiry: Expiry) {
    // A token that already fired (`stop_all`, a parent stop) is not an expiry (G1). Both
    // can still race in a tiny window; the only cost is a wrong reason text.
    if !timer.cancel.is_cancelled() {
        let _ = timer.expired.set(expiry);
    }
    timer.cancel.cancel();
    let _ = table.remove(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reaper::Check::{Fire, Wait};
    use crate::table::Entry;
    use crate::watch::MAX_BUDGET;
    use lazyrig_core::scope::Scope;
    use lazyrig_core::time::TickKind;
    use lazyrig_core::{pulse::Pulse, time::Budget};
    use tokio_util::sync::CancellationToken;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// A timer started at `t0` with a 10 s grace.
    fn new_timer(t0: Instant, hard: Option<u64>, idle: u64) -> Timer {
        let budget = Budget {
            timeout: hard.map(secs),
            idle_timeout: secs(idle),
        };
        Timer::new(Pulse::new(t0), budget, secs(10), CancellationToken::new())
    }

    #[test]
    fn the_rule() {
        let t0 = Instant::now();

        // Row 1: no beat, so stale = grace (10) + idle (300) = 310.
        let timer = new_timer(t0, Some(1800), 300);
        assert_eq!(check(t0 + secs(309), &timer), Wait(t0 + secs(310)), "row 1");

        // Row 2: the stale deadline is reached.
        assert_eq!(check(t0 + secs(310), &timer), Fire(Expiry::Stale), "row 2");

        // Row 3: a beat moves the stale deadline to beat + idle.
        let timer = new_timer(t0, Some(1800), 300);
        timer.pulse.beat_at(t0 + secs(200));
        assert_eq!(check(t0 + secs(400), &timer), Wait(t0 + secs(500)), "row 3");

        // Row 4: a hold pauses stale; look again at now + idle.
        let timer = new_timer(t0, Some(1800), 300);
        let _hold = timer.pulse.hold();
        assert_eq!(
            check(t0 + secs(1000), &timer),
            Wait(t0 + secs(1300)),
            "row 4"
        );

        // Row 5: a hold never pauses the hard timer.
        assert_eq!(
            check(t0 + secs(1800), &timer),
            Fire(Expiry::Timeout),
            "row 5"
        );

        // Row 6: no hard timer, stale only.
        let timer = new_timer(t0, None, 1800);
        timer.pulse.beat_at(t0 + secs(10_000));
        assert_eq!(
            check(t0 + secs(10_100), &timer),
            Wait(t0 + secs(11_800)),
            "row 6"
        );

        // Row 7: a huge idle budget is clamped when armed.
        let timer = new_timer(t0, None, u64::MAX / 1000);
        assert_eq!(check(t0, &timer), Wait(t0 + secs(10) + MAX_BUDGET), "row 7");

        // Row 8: the hard deadline (600) comes before the stale one (800).
        let timer = new_timer(t0, Some(600), 300);
        timer.pulse.beat_at(t0 + secs(500));
        assert_eq!(check(t0 + secs(550), &timer), Wait(t0 + secs(600)), "row 8");

        // Row 9: a zero idle budget while held still waits, so the reaper cannot spin.
        let timer = new_timer(t0, None, 0);
        let _hold = timer.pulse.hold();
        assert_eq!(check(t0, &timer), Wait(t0 + MIN_HELD_WAIT), "row 9");
    }

    #[tokio::test]
    async fn stale_timer_fires_the_token() {
        let table = Arc::new(Table::new());
        let id = table.insert(Entry {
            kind: TickKind::Shell,
            start: Instant::now(),
            owner: Scope::Session,
            tool: None,
            label: None,
            pulse: None,
        });
        let budget = Budget {
            timeout: None,
            idle_timeout: Duration::from_millis(50),
        };
        let timer = Arc::new(Timer::new(
            Pulse::new(Instant::now()),
            budget,
            Duration::ZERO,
            CancellationToken::new(),
        ));

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let reaper = tokio::spawn(run(rx, table.clone()));
        tx.send(Cmd::Arm(id, timer.clone())).unwrap();
        tokio::time::timeout(Duration::from_millis(150), timer.cancel.cancelled())
            .await
            .expect("fired in time");
        assert_eq!(timer.expired.get(), Some(&Expiry::Stale));
        assert_eq!(table.elapsed(id, Instant::now()), None);

        // Every sender gone: the reaper must return, not leak (D11).
        drop(tx);
        reaper.await.unwrap();
    }

    #[tokio::test]
    async fn disarm_never_fires() {
        let table = Arc::new(Table::new());
        let budget = Budget {
            timeout: None,
            idle_timeout: Duration::from_millis(50),
        };
        let timer = Arc::new(Timer::new(
            Pulse::new(Instant::now()),
            budget,
            Duration::ZERO,
            CancellationToken::new(),
        ));
        let id = table.insert(Entry {
            kind: TickKind::Shell,
            start: Instant::now(),
            owner: Scope::Session,
            tool: None,
            label: None,
            pulse: None,
        });

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let reaper = tokio::spawn(run(rx, table.clone()));
        tx.send(Cmd::Arm(id, timer.clone())).unwrap();
        tx.send(Cmd::Disarm(id)).unwrap();

        // Twice the stale budget: a live timer would have fired by now.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!timer.cancel.is_cancelled());
        assert_eq!(timer.expired.get(), None);

        drop(tx);
        reaper.await.unwrap();
    }
}
