//! The one map of running ticks and the id counter.

use lazyrig_core::id::ToolId;
use lazyrig_core::pulse::Pulse;
use lazyrig_core::scope::Scope;
use lazyrig_core::time::{Tick, TickId, TickKind};
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// One running tick.
pub(crate) struct Entry {
    /// What the tick measures.
    pub kind: TickKind,
    /// When the tick started.
    pub start: Instant,
    /// Who the tick belongs to. Never `Tool`: the tool rides in `tool`.
    pub owner: Scope,
    /// The tool call this tick times, if any.
    pub tool: Option<ToolId>,
    /// What a frontend row shows.
    pub label: Option<Arc<str>>,
    /// The activity signal of a watched tick. `None` for an elapsed-only tick.
    pub pulse: Option<Pulse>,
}

/// Every running tick by id, plus the counter that mints ids.
pub(crate) struct Table {
    ticks: RwLock<BTreeMap<TickId, Entry>>,
    next: AtomicU64,
}

impl Table {
    /// An empty map. The counter starts at 1 because a `TickId` is never 0.
    pub fn new() -> Self {
        Self {
            ticks: RwLock::new(BTreeMap::new()),
            next: AtomicU64::new(1),
        }
    }

    /// A new unique id. Ids are never reused, so an old id can never stop a new tick.
    fn next_id(&self) -> TickId {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let n = NonZeroU64::new(n).expect("tick counter starts at 1 and never wraps");
        TickId::new(n)
    }

    /// Add a tick and return its id. The id is minted before the lock.
    pub fn insert(&self, entry: Entry) -> TickId {
        let id = self.next_id();
        self.ticks.write().insert(id, entry);
        id
    }

    /// Take a tick out. `None` when it is already gone.
    pub fn remove(&self, id: TickId) -> Option<Entry> {
        self.ticks.write().remove(&id)
    }

    /// How long a tick has run at `now`. `None` when it is gone.
    pub fn elapsed(&self, id: TickId, now: Instant) -> Option<Duration> {
        self.ticks
            .read()
            .get(&id)
            .map(|entry| now.saturating_duration_since(entry.start))
    }

    /// Take out every tick of `owner`. The caller acts on them after the lock is gone.
    pub fn drain(&self, owner: Scope) -> Vec<Entry> {
        self.ticks
            .write()
            .extract_if(.., |_, entry| matches(owner, entry.owner))
            .map(|(_, entry)| entry)
            .collect()
    }

    /// Change a running tick's kind. A miss does nothing: the tick already ended.
    pub fn set_kind(&self, id: TickId, kind: TickKind) {
        if let Some(entry) = self.ticks.write().get_mut(&id) {
            entry.kind = kind;
        }
    }

    /// Refill `out` with one row per tick, in start order. One `now` for every row.
    pub fn fill(&self, out: &mut Vec<Tick>, now: Instant) {
        let ticks = self.ticks.read();
        out.clear();
        out.extend(ticks.iter().map(|(&id, entry)| Tick {
            id,
            kind: entry.kind,
            owner: entry.owner,
            tool: entry.tool,
            label: entry.label.clone(),
            elapsed: now.saturating_duration_since(entry.start),
            quiet: entry.pulse.as_ref().map(|pulse| quiet(pulse, now)),
        }));
    }
}

/// Time since the last activity at `now`. Zero while held: waiting on the user is not
/// stalling. Before the first beat it counts from the start.
fn quiet(pulse: &Pulse, now: Instant) -> Duration {
    if pulse.held() {
        return Duration::ZERO;
    }
    // A beat is never before the start, so this is `max(start, last_beat)`.
    let last = pulse.last_beat().unwrap_or(pulse.start());
    now.saturating_duration_since(last)
}

/// Exact match, with `Session` as the one wildcard. The ticker never walks the scope
/// tree: the agent calls `stop_all` for each child it closes.
fn matches(owner: Scope, tick_owner: Scope) -> bool {
    owner == Scope::Session || owner == tick_owner
}
