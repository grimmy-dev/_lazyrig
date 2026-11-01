//! What the application needs from the outside, as traits over core types.
//! The agent names these, never an adapter.

use std::{sync::Arc, time::Duration};

use lazyrig_core::{
    id::ToolId,
    pulse::Pulse,
    scope::Scope,
    time::{Expiry, TickId, TickKind, WatchKind},
};
use tokio_util::sync::CancellationToken;

/// Time for the application: ticks, and the timers on them. The ticker implements it.
pub trait Clock: Send + Sync {
    /// Start a tick now. `tool` is the tool call it times, if any. `label` is what a
    /// frontend row shows; `None` when no row shows the tick.
    #[must_use]
    fn start(
        &self,
        kind: TickKind,
        owner: Scope,
        tool: Option<ToolId>,
        label: Option<Arc<str>>,
    ) -> TickId;
    /// Remove a tick and return how long it ran. `None` when it is already stopped.
    fn stop(&self, id: TickId) -> Option<Duration>;
    /// How long a tick has run so far. `None` when it is stopped.
    #[must_use]
    fn elapsed(&self, id: TickId) -> Option<Duration>;
    /// Remove every tick of `owner` (`Session` = all) and return the elapsed of the
    /// owner's own tick, or zero. It never fires a token: the caller cancels its token
    /// tree first, the one kill path.
    fn stop_all(&self, owner: Scope) -> Duration;
    /// Start a tick with a hard and a stale timer. `timeout` is the model's ceiling, if it
    /// gave one: it can lower the configured one, never raise it. When a timer is spent,
    /// `cancel` fires and the caller kills its own work.
    #[must_use = "dropping the watch removes the tick and disarms its timers at once"]
    fn watch(
        &self,
        kind: WatchKind,
        owner: Scope,
        tool: Option<ToolId>,
        label: Option<Arc<str>>,
        timeout: Option<Duration>,
        cancel: CancellationToken,
    ) -> Box<dyn Watch>;
}

/// The handle a watched task holds. A trait and not a struct, because its drop reaches
/// back into the ticker: dropping it removes the tick and disarms without firing, so an
/// early return or a panic never leaves a timer armed.
pub trait Watch: Send {
    /// The watched tick.
    fn id(&self) -> TickId;
    /// The activity signal. Clone it into whatever reads the work's output.
    fn pulse(&self) -> Pulse;
    /// Why the token fired. `None` if it did not, or something else fired it.
    fn expired(&self) -> Option<Expiry>;
    /// Move to another kind's budget, keeping the start, the token and the model's
    /// timeout: a foreground shell moved to the background. It applies at the next
    /// deadline check, so a looser budget works at once and a tighter one waits for the
    /// old deadline.
    fn reclassify(&mut self, kind: WatchKind);
    /// End the watch without firing and return how long it ran. `Box<Self>` keeps the
    /// trait usable as `dyn Watch` and still consumes the handle.
    fn finish(self: Box<Self>) -> Duration;
}
