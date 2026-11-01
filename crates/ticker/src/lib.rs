//! Elapsed ticks and hard/stale timers for every running thing.
//! It fires a task's own token; it never kills and never runs caller code.

mod budget;
mod reaper;
mod table;
mod watch;

use lazyrig_core::id::ToolId;
use lazyrig_core::pulse::Pulse;
use lazyrig_core::scope::Scope;
use lazyrig_core::time::{Tick, TickId, TickKind, Timeouts, WatchKind};
use lazyrig_ports::{Clock, Watch};
use parking_lot::Mutex;
use reaper::Cmd;
use std::sync::Arc;
use std::time::{Duration, Instant};
use table::{Entry, Table};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;
use watch::{TickWatch, Timer};

struct Inner {
    /// Every running tick. Shared with the reaper, which removes a tick when it fires.
    table: Arc<Table>,
    /// Commands to the reaper. Unbounded, so `watch` and a drop never block.
    tx: UnboundedSender<Cmd>,
    /// Taken by the first `watch`, which spawns the reaper with it.
    rx: Mutex<Option<UnboundedReceiver<Cmd>>>,
    /// Read at each arm, so a config change applies to the next timer only.
    timeouts: Box<dyn Fn() -> Timeouts + Send + Sync>,
}

/// A cheap handle to the one tick map and its reaper. A clone is one refcount bump.
#[derive(Clone)]
pub struct Ticker(Arc<Inner>);

impl Ticker {
    /// An empty ticker. `timeouts` gives the current budgets; it is called each time a
    /// timer is armed. No task yet: the first `watch` spawns the reaper.
    #[must_use]
    pub fn new(timeouts: impl Fn() -> Timeouts + Send + Sync + 'static) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self(Arc::new(Inner {
            table: Arc::new(Table::new()),
            tx,
            rx: Mutex::new(Some(rx)),
            timeouts: Box::new(timeouts),
        }))
    }

    /// Refill `out` with one row per running tick, in start order. The frontend's only
    /// read, so it is not on the port.
    pub fn snapshot(&self, out: &mut Vec<Tick>) {
        self.0.table.fill(out, Instant::now());
    }
}

impl Clock for Ticker {
    fn start(
        &self,
        kind: TickKind,
        owner: Scope,
        tool: Option<ToolId>,
        label: Option<Arc<str>>,
    ) -> TickId {
        let entry = Entry {
            kind,
            owner,
            tool,
            label,
            start: Instant::now(),
            pulse: None,
        };
        self.0.table.insert(entry)
    }

    fn stop(&self, id: TickId) -> Option<Duration> {
        self.0.table.remove(id).map(|entry| entry.start.elapsed())
    }

    fn elapsed(&self, id: TickId) -> Option<Duration> {
        self.0.table.elapsed(id, Instant::now())
    }

    fn stop_all(&self, owner: Scope) -> Duration {
        let now = Instant::now();
        let own = TickKind::for_scope(owner);
        self.0
            .table
            .drain(owner)
            .iter()
            .find(|entry| Some(entry.kind) == own)
            .map_or(Duration::ZERO, |entry| {
                now.saturating_duration_since(entry.start)
            })
    }

    /// # Panics
    ///
    /// Outside a tokio runtime: the first call spawns the reaper.
    fn watch(
        &self,
        kind: WatchKind,
        owner: Scope,
        tool: Option<ToolId>,
        label: Option<Arc<str>>,
        timeout: Option<Duration>,
        cancel: CancellationToken,
    ) -> Box<dyn Watch> {
        // Its own statement, so the lock drops here and not at the end of the `if`.
        let rx = self.0.rx.lock().take();
        if let Some(rx) = rx {
            tokio::spawn(reaper::run(rx, Arc::clone(&self.0.table)));
        }

        let timeouts = (self.0.timeouts)();
        let budget = budget::budget(&timeouts, kind, timeout);
        // One start for the tick and its pulse.
        let start = Instant::now();
        let timer = Arc::new(Timer::new(
            Pulse::new(start),
            budget,
            timeouts.idle_grace,
            cancel,
        ));
        let id = self.0.table.insert(Entry {
            kind: kind.into(),
            start,
            owner,
            tool,
            label,
            pulse: Some(timer.pulse.clone()),
        });
        // The tick is in the table before the reaper knows of it, so a fire always finds it.
        let _ = self.0.tx.send(Cmd::Arm(id, Arc::clone(&timer)));

        Box::new(TickWatch {
            id,
            start,
            timer,
            model_timeout: timeout,
            ticker: self.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazyrig_core::id::SubSessionId;
    use lazyrig_core::time::{Budget, Expiry};
    use std::thread::sleep;

    const WAIT: Duration = Duration::from_millis(20);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn short_timeouts() -> Timeouts {
        Timeouts {
            idle_grace: Duration::ZERO,
            shell: Budget {
                timeout: None,
                idle_timeout: ms(50),
            },
            ..Timeouts::default()
        }
    }

    #[test]
    fn elapsed_grows_then_stop_ends_it() {
        let ticker = Ticker::new(Timeouts::default);
        let id = ticker.start(TickKind::Turn, Scope::Session, None, None);
        sleep(WAIT);
        assert!(ticker.elapsed(id) >= Some(WAIT));
        assert!(ticker.stop(id) >= Some(WAIT));
        assert_eq!(ticker.stop(id), None);
        assert_eq!(ticker.elapsed(id), None);
    }

    #[test]
    fn stop_all_takes_the_owner_and_its_tools() {
        let a: SubSessionId = "ss00000a".parse().unwrap();
        let b: SubSessionId = "ss00000b".parse().unwrap();
        let tool: ToolId = "t0000001".parse().unwrap();
        let ticker = Ticker::new(Timeouts::default);
        let _ = ticker.start(TickKind::Session, Scope::Session, None, None);
        let _ = ticker.start(TickKind::SubSession, Scope::SubSession(a), None, None);
        let _ = ticker.start(
            TickKind::Shell,
            Scope::SubSession(a),
            Some(tool),
            Some("ls".into()),
        );
        let b_tick = ticker.start(TickKind::SubSession, Scope::SubSession(b), None, None);
        sleep(WAIT);
        assert!(ticker.stop_all(Scope::SubSession(a)) >= WAIT);
        assert!(ticker.elapsed(b_tick).is_some());
        assert!(ticker.stop_all(Scope::Session) >= WAIT);
        assert!(ticker.elapsed(b_tick).is_none());
        assert_eq!(ticker.stop_all(Scope::Session), Duration::ZERO);
    }

    #[test]
    fn snapshot_fills_rows_in_start_order_without_growing() {
        let ticker = Ticker::new(Timeouts::default);
        let first = ticker.start(TickKind::Session, Scope::Session, None, None);
        let second = ticker.start(TickKind::Turn, Scope::Session, None, Some("hi".into()));
        let third = ticker.start(TickKind::Tools, Scope::Session, None, None);
        let mut rows = Vec::new();
        ticker.snapshot(&mut rows);
        let ids: Vec<TickId> = rows.iter().map(|row| row.id).collect();
        assert_eq!(ids, [first, second, third]);
        assert_eq!(rows[1].label.as_deref(), Some("hi"));
        let cap = rows.capacity();
        ticker.snapshot(&mut rows);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.capacity(), cap);
    }

    #[tokio::test]
    async fn unbeaten_watch_goes_stale() {
        let ticker = Ticker::new(short_timeouts);
        let token = CancellationToken::new();
        let busy = ticker.watch(
            WatchKind::Shell,
            Scope::Session,
            None,
            None,
            None,
            token.clone(),
        );

        // Beaten for 200 ms, four times the stale budget: still alive.
        let pulse = busy.pulse();
        for _ in 0..10 {
            pulse.beat();
            tokio::time::sleep(ms(20)).await;
        }
        assert!(!token.is_cancelled());

        // Silence: stale within the budget plus slack.
        tokio::time::timeout(ms(150), token.cancelled())
            .await
            .expect("went stale");
        assert_eq!(busy.expired(), Some(Expiry::Stale));

        // The reaper removed the tick when it fired.
        let mut rows = Vec::new();
        ticker.snapshot(&mut rows);
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn finish_ends_without_firing() {
        let ticker = Ticker::new(short_timeouts);
        let token = CancellationToken::new();
        let watch = ticker.watch(
            WatchKind::Shell,
            Scope::Session,
            None,
            None,
            None,
            token.clone(),
        );
        assert!(watch.finish() < ms(50));

        // Twice the stale budget: a live timer would have fired by now.
        tokio::time::sleep(ms(100)).await;
        assert!(!token.is_cancelled());
        let mut rows = Vec::new();
        ticker.snapshot(&mut rows);
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn watched_rows_report_quiet() {
        // Default budgets are long, so nothing fires here.
        let ticker = Ticker::new(Timeouts::default);
        let _turn = ticker.start(TickKind::Turn, Scope::Session, None, None);
        let watch = ticker.watch(
            WatchKind::Shell,
            Scope::Session,
            None,
            None,
            None,
            CancellationToken::new(),
        );
        tokio::time::sleep(ms(20)).await;

        // A plain tick has no timer; a watch with no beat counts from its start.
        let mut rows = Vec::new();
        ticker.snapshot(&mut rows);
        assert_eq!(rows[0].quiet, None);
        assert!(rows[1].quiet >= Some(ms(20)));

        // Held: waiting on the user, not stalled.
        let _hold = watch.pulse().hold();
        ticker.snapshot(&mut rows);
        assert_eq!(rows[1].quiet, Some(Duration::ZERO));
    }

    #[tokio::test]
    async fn reclassify_moves_to_the_new_budget() {
        // Shell idle is 50 ms; background keeps the default 1800 s.
        let ticker = Ticker::new(short_timeouts);
        let token = CancellationToken::new();
        let mut watch = ticker.watch(
            WatchKind::Shell,
            Scope::Session,
            None,
            None,
            None,
            token.clone(),
        );
        watch.reclassify(WatchKind::Background);

        // Three shell budgets: the lazy re-arm read the background budget at the old
        // deadline.
        tokio::time::sleep(ms(150)).await;
        assert!(!token.is_cancelled());
        let mut rows = Vec::new();
        ticker.snapshot(&mut rows);
        assert_eq!(rows[0].kind, TickKind::Background);
    }
}
